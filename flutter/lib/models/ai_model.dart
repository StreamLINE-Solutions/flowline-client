import 'dart:convert';
import 'dart:io';

import 'package:flutter_hbb/consts.dart';
import 'package:http/http.dart' as http;

import '../common.dart';
import '../utils/http_service.dart' as http_service;
import 'platform_model.dart';

/// 0092/0104 — rapport d'intervention IA (SovIA) + dictée STT.
///
/// Gating : la popup de fin de session et Settings > Report sont visibles
/// seulement si le tech est connecté ET que son compte a l'option IA
/// (`ai_enabled` serveur, mirror local `kOptionAiEnabled`). L'API refuse de
/// toute façon les clients sans opt-in.

final Map<SessionID, DateTime> _sessionStarts = {};
final Set<SessionID> _sessionReportShown = {};

void markSessionStart(SessionID sessionId) {
  _sessionStarts[sessionId] = DateTime.now();
  _sessionReportShown.remove(sessionId);
}

DateTime? sessionStartTime(SessionID sessionId) => _sessionStarts[sessionId];

/// Une seule popup de rapport par session (fermeture locale ou distante).
bool isSessionReportShown(SessionID sessionId) =>
    _sessionReportShown.contains(sessionId);

void markSessionReportShown(SessionID sessionId) =>
    _sessionReportShown.add(sessionId);

/// Annulation : la session continue, le rapport pourra être reproposé.
void clearSessionReportShown(SessionID sessionId) =>
    _sessionReportShown.remove(sessionId);

/// Langue de dictée : option du client si définie, sinon locale système
/// (normalisée en ISO-639-1 ; vide si indéterminable → le serveur décide).
///
/// Sans langue explicite, le modèle SovIA traduit la dictée au lieu de la
/// transcrire (constat 07/10) : on envoie donc toujours la langue du client.
String dictationLanguage() {
  var lang = bind.mainGetLocalOption(key: kCommConfKeyLang).trim().toLowerCase();
  if (lang.isEmpty || lang == 'default') {
    lang = localeName;
  }
  final code = lang.split(RegExp(r'[-_]')).first;
  return RegExp(r'^[a-z]{2}$').hasMatch(code) ? code : '';
}

String _serverError(http.Response resp) {
  try {
    final data = jsonDecode(utf8.decode(resp.bodyBytes));
    if (data is Map && data['error'] != null) {
      return data['error'].toString();
    }
  } catch (_) {}
  return 'HTTP ${resp.statusCode}';
}

/// Génère le rapport d'intervention (POST /api/ai/report).
/// Renvoie l'id du rapport stocké côté serveur et son texte.
Future<({int id, String report})> apiGenerateInterventionReport({
  required String notes,
  required List<String> tags,
  required String peerId,
  required String peerHostname,
  DateTime? startedAt,
  DateTime? endedAt,
}) async {
  final apiServer = await bind.mainGetApiServer();
  if (apiServer.isEmpty) {
    throw Exception('API FlowLINE non configurée');
  }
  final headers = getHttpHeaders();
  headers['Content-Type'] = 'application/json';
  final body = jsonEncode({
    'notes': notes,
    'tags': tags,
    'peer_id': peerId,
    'peer_hostname': peerHostname,
    'started_at': startedAt?.toUtc().toIso8601String() ?? '',
    'ended_at': endedAt?.toUtc().toIso8601String() ?? '',
    'duration_seconds':
        (startedAt != null && endedAt != null)
            ? endedAt.difference(startedAt).inSeconds
            : 0,
  });
  final resp = await http_service.post(
    Uri.parse('$apiServer/api/ai/report'),
    headers: headers,
    body: body,
  );
  if (resp.statusCode == 200) {
    final data = jsonDecode(utf8.decode(resp.bodyBytes));
    return (id: data['report_id'] as int, report: data['report'] as String);
  }
  throw Exception(_serverError(resp));
}

/// Dossier de sauvegarde des PDF de rapport : option client si définie,
/// sinon `~/Documents/FlowLINE`.
String reportDirectory() {
  final configured =
      bind.mainGetLocalOption(key: kOptionReportSaveDirectory).trim();
  if (configured.isNotEmpty) {
    return configured;
  }
  final home = Platform.environment['HOME'] ?? Directory.systemTemp.path;
  return '$home/Documents/FlowLINE';
}

/// Télécharge le PDF du rapport et le sauvegarde dans le dossier configuré.
/// Renvoie le chemin du fichier écrit.
///
/// Le PDF arrive en base64 dans un JSON : le transport HTTP du client (chemin
/// Rust, proxys SOCKS/TCP) convertit les corps binaires en texte lossy, ce qui
/// corrompt un PDF brut (rejeté ensuite par l'encodage latin1 côté Dart).
Future<String> apiSaveInterventionPdf(int reportId) async {
  final apiServer = await bind.mainGetApiServer();
  if (apiServer.isEmpty) {
    throw Exception('API FlowLINE non configurée');
  }
  final resp = await http_service.get(
    Uri.parse('$apiServer/api/ai/reports/$reportId/pdf'),
    headers: getHttpHeaders(),
  );
  if (resp.statusCode != 200) {
    throw Exception(_serverError(resp));
  }
  final data = jsonDecode(utf8.decode(resp.bodyBytes));
  final bytes = base64Decode(data['content_base64'] as String);
  final name = (data['filename'] as String?) ?? 'FlowLINE_CR_$reportId.pdf';
  final dir = reportDirectory();
  await Directory(dir).create(recursive: true);
  final path = '$dir/$name';
  await File(path).writeAsBytes(bytes);
  return path;
}

/// Transcrit un fichier audio (POST /api/ai/transcribe, multipart, `sovia-stt`).
Future<String> apiTranscribeAudio(String filePath) async {
  final apiServer = await bind.mainGetApiServer();
  if (apiServer.isEmpty) {
    throw Exception('API FlowLINE non configurée');
  }
  final request = http.MultipartRequest(
    'POST',
    Uri.parse('$apiServer/api/ai/transcribe'),
  );
  request.headers.addAll(getHttpHeaders());
  // Langue de l'interface : force la langue de transcription (sinon le
  // modèle traduit la dictée au lieu de la transcrire).
  final lang = dictationLanguage();
  if (lang.isNotEmpty) {
    request.fields['language'] = lang;
  }
  request.files.add(await http.MultipartFile.fromPath('file', filePath));
  final streamed = await request.send();
  final resp = await http.Response.fromStream(streamed);
  if (resp.statusCode == 200) {
    final data = jsonDecode(utf8.decode(resp.bodyBytes));
    return data['text'] as String;
  }
  throw Exception(_serverError(resp));
}