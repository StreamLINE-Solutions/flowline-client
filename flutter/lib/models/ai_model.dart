import 'dart:convert';

import 'package:http/http.dart' as http;

import '../common.dart';
import '../utils/http_service.dart' as http_service;
import 'platform_model.dart';

/// POC 0092 — rapport d'intervention IA (SovIA) + dictée STT.
///
/// Actif uniquement sur les builds dev (API locale `http://localhost`) tant
/// que la fonctionnalité n'est pas ouverte en production (le gating opt-in
/// viendra ensuite — l'API refuse déjà les clients sans opt-in).

final Map<SessionID, DateTime> _sessionStarts = {};

void markSessionStart(SessionID sessionId) {
  _sessionStarts[sessionId] = DateTime.now();
}

DateTime? sessionStartTime(SessionID sessionId) => _sessionStarts[sessionId];

/// Build POC : API locale (dev). Les builds de production ne déclenchent pas
/// la popup de rapport pour l'instant.
Future<bool> isPocAiBuild() async {
  final apiServer = await bind.mainGetApiServer();
  return apiServer.startsWith('http://localhost') ||
      apiServer.startsWith('http://127.0.0.1');
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

/// Génère le rapport d'intervention (POST /api/ai/report) et renvoie son texte.
Future<String> apiGenerateInterventionReport({
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
    return data['report'] as String;
  }
  throw Exception(_serverError(resp));
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
  request.files.add(await http.MultipartFile.fromPath('file', filePath));
  final streamed = await request.send();
  final resp = await http.Response.fromStream(streamed);
  if (resp.statusCode == 200) {
    final data = jsonDecode(utf8.decode(resp.bodyBytes));
    return data['text'] as String;
  }
  throw Exception(_serverError(resp));
}