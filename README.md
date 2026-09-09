# FlowLINE

FlowLINE est un client de contrôle à distance pour Windows, macOS, Linux et
Android, distribué en mode technicien (accès non surveillé) et en mode support
(QuickSupport, accès sous contrôle de la personne dépannée).

Ce projet est un **white-label de RustDesk** (AGPL-3.0). Voir `LICENSE` et
`NOTICE.md`.

## Fonctionnalités

- Contrôle à distance multi-plateformes (Windows, macOS, Linux, Android, iOS).
- Mode technicien et mode support (QuickSupport).
- Serveurs de rendez-vous / relais auto-hébergés.
- Transfert de fichiers, partage d'écran, presse-papiers, audio.

## Compilation

Le build est déclenché via les workflows GitHub Actions (`.github/workflows/`) :

- `flowline-windows.yml` — Windows (MSI + one-file support)
- `flowline-linux.yml` — Linux (.deb + AppImage)
- `flowline-macos.yml` — macOS (dmg)
- `flowline-android-support.yml` — Android (APK)

Builds manuels : `./build.py --flutter --hwcodec --unix-file-copy-paste` (voir
les docs du projet RustDesk d'origine pour le détail).

## Configuration de l'infrastructure

Les serveurs de rendez-vous et la clé publique de FlowLINE sont définis dans
les workflows via les variables `RUSTDESK_RENDEZVOUS_SERVERS` et
`RUSTDESK_RS_PUB_KEY`. Déployer vers une autre infrastructure = surcharger ces
variables au moment du build.

## Licence

GNU Affero General Public License v3.0 — voir `LICENSE`.

Le nom « FlowLINE » et le logo sont la propriété de StreamLINE Solutions et ne
sont pas couverts par l'AGPL-3.0 (voir `NOTICE.md`).