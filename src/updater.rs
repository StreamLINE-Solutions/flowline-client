use crate::{common::do_check_software_update, hbbs_http::create_http_client_with_url};
use hbb_common::{anyhow, bail, config, log, ResultType};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    time::{Duration, Instant},
};

enum UpdateMsg {
    CheckUpdate,
    Exit,
}

lazy_static::lazy_static! {
    static ref TX_MSG : Mutex<Sender<UpdateMsg>> = Mutex::new(start_auto_update_check());
}

static CONTROLLING_SESSION_COUNT: AtomicUsize = AtomicUsize::new(0);

const DUR_ONE_DAY: Duration = Duration::from_secs(60 * 60 * 24);

/// 0072 : base des artefacts publiés (site FlowLINE).
#[cfg(target_os = "linux")]
const FLOWLINE_DOWNLOADS_BASE: &str = "https://flowline.my-vth.ch/downloads";
/// 0072 : page de téléchargement (fallback AppImage / architecture non couverte).
#[cfg(target_os = "linux")]
const FLOWLINE_DOWNLOAD_PAGE: &str = "https://flowline.my-vth.ch/download.html";

pub fn update_controlling_session_count(count: usize) {
    CONTROLLING_SESSION_COUNT.store(count, Ordering::SeqCst);
}

#[allow(dead_code)]
pub fn start_auto_update() {
    let _sender = TX_MSG.lock().unwrap();
}

#[allow(dead_code)]
pub fn manually_check_update() -> ResultType<()> {
    let sender = TX_MSG.lock().unwrap();
    sender.send(UpdateMsg::CheckUpdate)?;
    Ok(())
}

#[allow(dead_code)]
pub fn stop_auto_update() {
    let sender = TX_MSG.lock().unwrap();
    sender.send(UpdateMsg::Exit).unwrap_or_default();
}

#[inline]
fn has_no_active_conns() -> bool {
    let conns = crate::Connection::alive_conns();
    conns.is_empty() && has_no_controlling_conns()
}

#[cfg(any(not(target_os = "windows"), feature = "flutter"))]
fn has_no_controlling_conns() -> bool {
    CONTROLLING_SESSION_COUNT.load(Ordering::SeqCst) == 0
}

#[cfg(not(any(not(target_os = "windows"), feature = "flutter")))]
fn has_no_controlling_conns() -> bool {
    let app_exe = format!("{}.exe", crate::get_app_name().to_lowercase());
    for arg in [
        "--connect",
        "--play",
        "--file-transfer",
        "--view-camera",
        "--port-forward",
        "--rdp",
    ] {
        if !crate::platform::get_pids_of_process_with_first_arg(&app_exe, arg).is_empty() {
            return false;
        }
    }
    true
}

fn start_auto_update_check() -> Sender<UpdateMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || start_auto_update_check_(rx));
    return tx;
}

fn start_auto_update_check_(rx_msg: Receiver<UpdateMsg>) {
    std::thread::sleep(Duration::from_secs(30));
    if let Err(e) = check_update(false) {
        log::error!("Error checking for updates: {}", e);
    }

    const MIN_INTERVAL: Duration = Duration::from_secs(60 * 10);
    const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 30);
    let mut last_check_time = Instant::now();
    let mut check_interval = DUR_ONE_DAY;
    loop {
        let recv_res = rx_msg.recv_timeout(check_interval);
        match &recv_res {
            Ok(UpdateMsg::CheckUpdate) | Err(_) => {
                if last_check_time.elapsed() < MIN_INTERVAL {
                    // log::debug!("Update check skipped due to minimum interval.");
                    continue;
                }
                // Don't check update if there are alive connections.
                if !has_no_active_conns() {
                    check_interval = RETRY_INTERVAL;
                    continue;
                }
                if let Err(e) = check_update(matches!(recv_res, Ok(UpdateMsg::CheckUpdate))) {
                    log::error!("Error checking for updates: {}", e);
                    check_interval = RETRY_INTERVAL;
                } else {
                    last_check_time = Instant::now();
                    check_interval = DUR_ONE_DAY;
                }
            }
            Ok(UpdateMsg::Exit) => break,
        }
    }
}

fn check_update(manually: bool) -> ResultType<()> {
    #[cfg(target_os = "windows")]
    // FlowLINE : on distribue un MSI, y compris pour le client custom (le check
    // upstream force `exe` pour les custom clients, ce qui n'est pas notre cas).
    let update_msi = crate::platform::is_msi_installed()?;
    if !(manually || config::Config::get_bool_option(config::keys::OPTION_ALLOW_AUTO_UPDATE)) {
        return Ok(());
    }
    if do_check_software_update().is_err() {
        // ignore
        return Ok(());
    }

    let update_url = crate::common::SOFTWARE_UPDATE_URL.lock().unwrap().clone();
    if update_url.is_empty() {
        log::debug!("No update available.");
    } else {
        let download_url = update_url.replace("tag", "download");
        let version = download_url.split('/').last().unwrap_or_default();
        #[cfg(target_os = "windows")]
        let download_url = if cfg!(feature = "flutter") {
            let Some(arch) = crate::platform::windows::release_arch_suffix() else {
                bail!(
                    "Unsupported Windows release architecture: {}",
                    std::env::consts::ARCH
                );
            };
            format!(
                "{}/{}-{}-{}.{}",
                download_url,
                crate::get_app_name().to_lowercase(),
                version,
                arch,
                if update_msi { "msi" } else { "exe" }
            )
        } else {
            format!("{}/rustdesk-{}-x86-sciter.exe", download_url, version)
        };
        log::debug!("New version available: {}", &version);
        let client = create_http_client_with_url(&download_url);
        let Some(file_path) = get_download_file_from_url(&download_url) else {
            bail!("Failed to get the file path from the URL: {}", download_url);
        };
        let mut is_file_exists = false;
        if file_path.exists() {
            // Check if the file size is the same as the server file size
            // If the file size is the same, we don't need to download it again.
            let file_size = std::fs::metadata(&file_path)?.len();
            let response = client.head(&download_url).send()?;
            if !response.status().is_success() {
                bail!("Failed to get the file size: {}", response.status());
            }
            let total_size = response
                .headers()
                .get(reqwest::header::CONTENT_LENGTH)
                .and_then(|ct_len| ct_len.to_str().ok())
                .and_then(|ct_len| ct_len.parse::<u64>().ok());
            let Some(total_size) = total_size else {
                bail!("Failed to get content length");
            };
            if file_size == total_size {
                is_file_exists = true;
            } else {
                std::fs::remove_file(&file_path)?;
            }
        }
        if !is_file_exists {
            let response = client.get(&download_url).send()?;
            if !response.status().is_success() {
                bail!(
                    "Failed to download the new version file: {}",
                    response.status()
                );
            }
            let file_data = response.bytes()?;
            let mut file = std::fs::File::create(&file_path)?;
            file.write_all(&file_data)?;
        }
        // 0050 : vérifier le manifeste signé (Ed25519, clé embarquée) et le
        // SHA-256 du fichier AVANT toute installation. Une API/NPM compromise ne
        // peut pas forger de mise à jour. Échec => fichier supprimé, pas
        // d'installation (fail closed).
        #[cfg(target_os = "windows")]
        if update_msi {
            if let Err(e) = fetch_and_verify_update_manifest(&download_url, version, &file_path) {
                std::fs::remove_file(&file_path).ok();
                bail!("Mise à jour refusée (manifeste invalide): {e}");
            }
        }
        // We have checked if the `conns` is empty before, but we need to check again.
        // No need to care about the downloaded file here, because it's rare case that the `conns` are empty
        // before the download, but not empty after the download.
        if has_no_active_conns() {
            #[cfg(target_os = "windows")]
            update_new_version(update_msi, &version, &file_path);
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn update_new_version(update_msi: bool, version: &str, file_path: &PathBuf) {
    log::debug!(
        "New version is downloaded, update begin, update msi: {update_msi}, version: {version}, file: {:?}",
        file_path.to_str()
    );
    if let Some(p) = file_path.to_str() {
        if let Some(session_id) = crate::platform::get_current_process_session_id() {
            if update_msi {
                match crate::platform::update_me_msi(p, true) {
                    Ok(_) => {
                        log::debug!("New version \"{}\" updated.", version);
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to install the new msi version  \"{}\": {}",
                            version,
                            e
                        );
                        std::fs::remove_file(&file_path).ok();
                    }
                }
            } else {
                let custom_client_staging_dir = if crate::is_custom_client() {
                    let custom_client_staging_dir =
                        crate::platform::get_custom_client_staging_dir();
                    if let Err(e) = crate::platform::handle_custom_client_staging_dir_before_update(
                        &custom_client_staging_dir,
                    ) {
                        log::error!(
                            "Failed to handle custom client staging dir before update: {}",
                            e
                        );
                        std::fs::remove_file(&file_path).ok();
                        return;
                    }
                    Some(custom_client_staging_dir)
                } else {
                    // Clean up any residual staging directory from previous custom client
                    let staging_dir = crate::platform::get_custom_client_staging_dir();
                    hbb_common::allow_err!(crate::platform::remove_custom_client_staging_dir(
                        &staging_dir
                    ));
                    None
                };
                let update_launched = match crate::platform::launch_privileged_process(
                    session_id,
                    &format!("{} --update", p),
                ) {
                    Ok(h) => {
                        if h.is_null() {
                            log::error!("Failed to update to the new version: {}", version);
                            false
                        } else {
                            log::debug!("New version \"{}\" is launched.", version);
                            true
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to run the new version: {}", e);
                        false
                    }
                };
                if !update_launched {
                    if let Some(dir) = custom_client_staging_dir {
                        hbb_common::allow_err!(crate::platform::remove_custom_client_staging_dir(
                            &dir
                        ));
                    }
                    std::fs::remove_file(&file_path).ok();
                }
            }
        } else {
            log::error!(
                "Failed to get the current process session id, Error {}",
                std::io::Error::last_os_error()
            );
            std::fs::remove_file(&file_path).ok();
        }
    } else {
        // unreachable!()
        log::error!(
            "Failed to convert the file path to string: {}",
            file_path.display()
        );
    }
}

pub fn get_download_file_from_url(url: &str) -> Option<PathBuf> {
    let filename = url.split('/').last()?;
    Some(std::env::temp_dir().join(filename))
}

/// 0050 : extrait la version depuis l'URL de téléchargement du MSI
/// (`.../api/update/download/<version>/<fichier>`).
#[allow(dead_code)]
pub fn version_from_download_url(download_url: &str) -> Option<&str> {
    let rest = download_url.split_once("/download/")?.1;
    let version = rest.split('/').next()?;
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// 0050 : dérive l'URL du manifeste signé depuis celle du MSI
/// (`.../api/update/download/<version>/...` → `.../api/update/manifest/<version>`).
#[allow(dead_code)]
fn manifest_url_from_download_url(download_url: &str) -> Option<String> {
    let (prefix, _) = download_url.split_once("/download/")?;
    let version = version_from_download_url(download_url)?;
    Some(format!("{prefix}/manifest/{version}"))
}

/// 0050 : récupère le manifeste signé servi par l'API et le vérifie contre le
/// fichier téléchargé (key_id/clé embarquée, signature Ed25519, version, nom,
/// taille, SHA-256). Toute anomalie => erreur (l'appelant refuse et supprime).
#[allow(dead_code)]
pub fn fetch_and_verify_update_manifest(
    download_url: &str,
    version: &str,
    file_path: &Path,
) -> ResultType<()> {
    let manifest_url = manifest_url_from_download_url(download_url)
        .ok_or_else(|| anyhow::anyhow!("URL de manifeste introuvable: {download_url}"))?;
    let client = create_http_client_with_url(&manifest_url);
    let response = client.get(&manifest_url).send()?;
    if !response.status().is_success() {
        bail!(
            "manifeste de mise à jour indisponible: {}",
            response.status()
        );
    }
    let envelope: hbb_common::update_manifest::UpdateManifestEnvelope =
        serde_json::from_slice(&response.bytes()?)?;
    hbb_common::update_manifest::verify_update_manifest(&envelope, version, file_path)?;
    log::debug!(
        "Manifeste de mise à jour vérifié (version {}, fichier {:?})",
        version,
        file_path
    );
    Ok(())
}

/// 0050 : vérifie un fichier déjà téléchargé (chemin manuel de mise à jour,
/// `update-me` côté UI) avant de lancer l'installation.
#[allow(dead_code)]
pub fn verify_downloaded_update(download_url: &str, file_path: &Path) -> ResultType<()> {
    let version = version_from_download_url(download_url)
        .ok_or_else(|| anyhow::anyhow!("version introuvable dans l'URL de mise à jour"))?;
    fetch_and_verify_update_manifest(download_url, version, file_path)
}
/// 0072 : nom du paquet deb du client technicien (seule l'architecture x86_64
/// est publiée à ce jour).
#[cfg(target_os = "linux")]
fn linux_deb_filename(version: &str) -> Option<String> {
    if std::env::consts::ARCH == "x86_64" {
        Some(format!("flowline-{version}-x86_64.deb"))
    } else {
        None
    }
}

/// 0072 (Option B) : télécharge le .deb de `version` depuis le site FlowLINE,
/// vérifie le manifeste de release signé (Ed25519, clé embarquée — 0050 étendu)
/// et l'artefact (nom, taille, SHA-256), puis retourne le chemin du fichier
/// vérifié. Toute anomalie => fichier supprimé + erreur (fail closed).
#[cfg(target_os = "linux")]
pub fn download_verified_linux_deb(version: &str) -> ResultType<PathBuf> {
    let file_name = linux_deb_filename(version).ok_or_else(|| {
        anyhow::anyhow!(
            "aucun paquet Linux publié pour l'architecture {}",
            std::env::consts::ARCH
        )
    })?;
    let download_url = format!("{FLOWLINE_DOWNLOADS_BASE}/{file_name}");
    let path = std::env::temp_dir().join(&file_name);
    let client = create_http_client_with_url(&download_url);
    let mut need_download = true;
    if path.exists() {
        // Même taille que sur le serveur => pas besoin de retélécharger (la
        // vérification signature/hash est faite dans tous les cas ci-dessous).
        let file_size = std::fs::metadata(&path)?.len();
        let response = client.head(&download_url).send()?;
        if !response.status().is_success() {
            bail!("fichier de mise à jour indisponible: {}", response.status());
        }
        let total_size = response
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|ct_len| ct_len.to_str().ok())
            .and_then(|ct_len| ct_len.parse::<u64>().ok());
        if total_size == Some(file_size) {
            need_download = false;
        }
    }
    if need_download {
        let response = client.get(&download_url).send()?;
        if !response.status().is_success() {
            bail!(
                "téléchargement de la mise à jour impossible: {}",
                response.status()
            );
        }
        std::fs::write(&path, &response.bytes()?)?;
    }
    let manifest_url = format!("{FLOWLINE_DOWNLOADS_BASE}/flowline-{version}.manifest.json");
    let response = client.get(&manifest_url).send()?;
    if !response.status().is_success() {
        bail!("manifeste de release indisponible: {}", response.status());
    }
    let manifest = response.bytes()?;
    let response = client.get(&format!("{manifest_url}.sig")).send()?;
    if !response.status().is_success() {
        bail!(
            "signature du manifeste de release indisponible: {}",
            response.status()
        );
    }
    let sig = response.bytes()?;
    if let Err(e) =
        hbb_common::update_manifest::verify_release_manifest(&manifest, &sig, version, &path)
    {
        std::fs::remove_file(&path).ok();
        bail!("mise à jour refusée (manifeste de release invalide): {e}");
    }
    log::debug!(
        "Mise à jour Linux {} téléchargée et vérifiée: {:?}",
        version,
        path
    );
    Ok(path)
}

/// 0072 : ouvre une cible (URL ou fichier) avec le gestionnaire par défaut.
#[cfg(target_os = "linux")]
fn xdg_open(target: &str) -> ResultType<()> {
    std::process::Command::new("xdg-open").arg(target).spawn()?;
    Ok(())
}

/// 0072 (Option B) / 1.4.21 (v2) : ouvre la mise à jour Linux — télécharge et
/// vérifie le .deb puis l'installe via pkexec/apt-get (prompt polkit) ; repli
/// sur l'ouverture de l'installateur système si pkexec est absent ; AppImage
/// ou installation non système => ouvre la page de téléchargement du site.
#[cfg(target_os = "linux")]
pub fn open_linux_update() -> ResultType<()> {
    if std::env::var_os("APPIMAGE").is_some() || !Path::new("/usr/bin/flowline").exists() {
        xdg_open(FLOWLINE_DOWNLOAD_PAGE)?;
        return Ok(());
    }
    let version = crate::common::SOFTWARE_UPDATE_URL
        .lock()
        .unwrap()
        .split('/')
        .last()
        .map(|s| s.to_owned())
        .unwrap_or_default();
    if version.is_empty() {
        xdg_open(FLOWLINE_DOWNLOAD_PAGE)?;
        return Ok(());
    }
    let path = download_verified_linux_deb(&version)?;
    install_linux_deb(&path)
}

/// 1.4.21 (Option B v2) : installe le .deb vérifié via pkexec + apt-get
/// (fenêtre de mot de passe polkit) ; si pkexec est absent, repli sur
/// l'ouverture du fichier par l'installateur système (0072). Le résultat est
/// remonté à l'UI Flutter via l'événement `flowline_update_install_finish`
/// (status ok/error) — cf. `checkUpdate()` côté Dart.
#[cfg(target_os = "linux")]
fn install_linux_deb(path: &Path) -> ResultType<()> {
    const PKEXEC: &str = "/usr/bin/pkexec";
    const APT_GET: &str = "/usr/bin/apt-get";
    if !Path::new(PKEXEC).exists() || !Path::new(APT_GET).exists() {
        return xdg_open(&path.to_string_lossy());
    }
    let status = std::process::Command::new(PKEXEC)
        .arg(APT_GET)
        .args(["install", "-y"])
        .arg(path)
        .status();
    let (ok, message) = match status {
        Ok(s) if s.success() => (true, "installed".to_owned()),
        Ok(s) => (
            false,
            format!(
                "apt-get exited with {}",
                s.code()
                    .map_or_else(|| "signal".to_owned(), |code| code.to_string())
            ),
        ),
        Err(e) => (false, e.to_string()),
    };
    log::info!(
        "Mise à jour Linux: installation de {} via pkexec/apt-get: {}",
        path.display(),
        message
    );
    #[cfg(feature = "flutter")]
    {
        let mut m = std::collections::HashMap::new();
        m.insert("name", "flowline_update_install_finish");
        m.insert("status", if ok { "ok" } else { "error" });
        m.insert("message", message.as_str());
        if let Ok(data) = serde_json::to_string(&m) {
            let _ = crate::flutter::push_global_event(crate::flutter::APP_TYPE_MAIN, data);
        }
    }
    if ok {
        Ok(())
    } else {
        bail!("installation de la mise à jour Linux échouée: {message}")
    }
}
