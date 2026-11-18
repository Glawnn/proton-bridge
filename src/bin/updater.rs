use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::Duration;

use crate::update::{
    UpdateClient, UpdateState, read_update_state, update_download_dir, update_state_path,
    write_update_state,
};

#[path = "../update.rs"]
mod update;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        anyhow::bail!(
            "Usage: proton-bridge-updater.exe --check [--current-version X.Y.Z] [--main-exe chemin] | --do-update [--main-exe chemin]"
        );
    }

    let command = &args[1];

    match command.as_str() {
        "--check" => {
            let current_version = get_arg(&args, "--current-version")
                .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
            let _main_exe = get_arg(&args, "--main-exe");

            let client = UpdateClient::new()?;
            match client.check_for_update(&current_version).await? {
                Some(release) => {
                    println!(
                        "Mise a jour disponible : {} (actuelle : {})",
                        release.version, current_version
                    );

                    let download_dir = update_download_dir()?;
                    let download_path = download_dir.join("proton-bridge.exe");

                    client
                        .download(&release.download_url, &download_path)
                        .await?;

                    let state = UpdateState {
                        pending_update: true,
                        new_version: Some(release.version.clone()),
                        download_path: Some(download_path.to_string_lossy().to_string()),
                    };
                    write_update_state(&state)?;

                    println!(
                        "Mise a jour telechargee dans {:?}. Redemarrez proton-bridge pour l'appliquer.",
                        download_path
                    );
                }
                None => {
                    println!(
                        "Aucune mise a jour disponible (version {})",
                        current_version
                    );
                    // Nettoyer tout etat precedent
                    let _ = std::fs::remove_file(update_state_path()?);
                }
            }
        }
        "--do-update" => {
            let main_exe =
                get_arg(&args, "--main-exe").context("--main-exe est requis avec --do-update")?;

            let state = read_update_state()?;
            if !state.pending_update {
                println!("Aucune mise a jour en attente.");
                return Ok(());
            }

            let new_version = state.new_version.as_deref().unwrap_or("inconnue");
            let download_path = state
                .download_path
                .as_deref()
                .context("Chemin de telechargement manquant dans l'etat de mise a jour")?;

            println!("Application de la mise a jour {} ...", new_version);

            let exe_path = PathBuf::from(main_exe);
            if !exe_path.exists() {
                anyhow::bail!("L'executable cible {:?} est introuvable", exe_path);
            }

            // Sous Windows, on ne peut pas ecraser un .exe en cours d'execution.
            // Strategie : renommer l'actuel en .old, deplacer le nouveau, lancer le nouveau.
            let old_path = exe_path.with_extension("exe.old");

            // Supprimer un ancien .old s'il existe
            if old_path.exists() {
                std::fs::remove_file(&old_path)
                    .context("Impossible de supprimer l'ancien .exe.old")?;
            }

            // Renommer l'actuel en .old
            std::fs::rename(&exe_path, &old_path)
                .context("Impossible de renommer l'executable actuel en .old")?;

            // Deplacer le nouveau a la place de l'actuel
            std::fs::rename(download_path, &exe_path)
                .context("Impossible de deplacer le nouvel executable a sa place")?;

            // Marquer l'etat comme applique
            let applied_state = UpdateState {
                pending_update: false,
                new_version: Some(new_version.to_string()),
                download_path: None,
            };
            write_update_state(&applied_state)?;

            println!("Mise a jour appliquee. Redemarrage de proton-bridge...");

            // Lancer la nouvelle version
            Command::new(&exe_path)
                .spawn()
                .context("Impossible de relancer proton-bridge apres la mise a jour")?;

            // Supprimer l'ancien .old apres un court delai (le nouveau exe est lance)
            thread::sleep(Duration::from_secs(2));
            if old_path.exists() {
                let _ = std::fs::remove_file(&old_path);
            }

            println!("Mise a jour terminee avec succes.");
        }
        _ => {
            anyhow::bail!("Commande inconnue : {}", command);
        }
    }

    Ok(())
}

/// Recupere la valeur d'un argument nomme (--nom valeur).
fn get_arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find_map(|pair| {
        if pair[0] == name {
            Some(pair[1].clone())
        } else {
            None
        }
    })
}
