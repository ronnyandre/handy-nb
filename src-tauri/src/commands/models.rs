use crate::managers::custom_models::{self, HfRepoListing, NewCustomModel};
use crate::managers::model::{ModelInfo, ModelManager};
use crate::managers::transcription::{ModelStateEvent, TranscriptionManager};
use crate::settings::{
    get_settings, write_settings, AppSettings, CustomModelDef, ModelUnloadTimeout,
};
use log::error;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

#[tauri::command]
#[specta::specta]
pub async fn get_available_models(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<Vec<ModelInfo>, String> {
    Ok(model_manager.get_available_models())
}

#[tauri::command]
#[specta::specta]
pub async fn get_model_info(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<Option<ModelInfo>, String> {
    Ok(model_manager.get_model_info(&model_id))
}

/// Re-scan local sources (custom models dir + shared HF cache) for models added
/// since launch
#[tauri::command]
#[specta::specta]
pub async fn rescan_local_models(
    model_manager: State<'_, Arc<ModelManager>>,
) -> Result<(), String> {
    let mm = model_manager.inner().clone();
    tokio::task::spawn_blocking(move || mm.rescan_local_models())
        .await
        .map_err(|e| format!("rescan task panicked: {e}"))?
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn download_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<(), String> {
    let result = model_manager
        .download_model(&model_id)
        .await
        .and_then(|()| {
            if is_user_defined(&model_id) {
                model_manager.verify_user_defined_download(&model_id)
            } else {
                Ok(())
            }
        })
        .map_err(|e| e.to_string());

    if let Err(ref error) = result {
        // Log as well as emit: the toast is transient, and failed downloads have
        // historically been undiagnosable because logs showed nothing (#1579).
        error!("Model download failed for {}: {}", model_id, error);
        let _ = app_handle.emit(
            "model-download-failed",
            serde_json::json!({ "model_id": &model_id, "error": error }),
        );
    }

    result
}

#[tauri::command]
#[specta::specta]
pub async fn delete_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    model_id: String,
) -> Result<(), String> {
    // If deleting the active model, unload it and clear the setting
    let settings = get_settings(&app_handle);
    if settings.selected_model == model_id {
        transcription_manager
            .unload_model()
            .map_err(|e| format!("Failed to unload model: {}", e))?;

        let mut settings = get_settings(&app_handle);
        settings.selected_model = String::new();
        write_settings(&app_handle, settings);
    }

    model_manager
        .delete_model(&model_id)
        .map_err(|e| e.to_string())
}

/// Shared logic for switching the active model, used by both the Tauri command
/// and the tray menu handler.
///
/// Validates the model, updates the persisted setting, and loads the model
/// unless the unload timeout is set to "Immediately" (in which case the model
/// will be loaded on-demand during the next transcription).
pub fn switch_active_model(app: &AppHandle, model_id: &str) -> Result<(), String> {
    let model_manager = app.state::<Arc<ModelManager>>();
    let transcription_manager = app.state::<Arc<TranscriptionManager>>();

    // Atomically claim the loading slot — prevents concurrent model loads
    // from tray double-clicks or overlapping commands. The guard resets the
    // flag on drop (including early returns, errors, and panics).
    let _loading_guard = transcription_manager
        .try_start_loading()
        .ok_or_else(|| "Model load already in progress".to_string())?;

    // Check if model exists and is available
    let model_info = model_manager
        .get_model_info(model_id)
        .ok_or_else(|| format!("Model not found: {}", model_id))?;

    if !model_info.is_downloaded {
        return Err(format!("Model not downloaded: {}", model_id));
    }

    let settings = get_settings(app);
    let unload_timeout = settings.model_unload_timeout;
    let old_model = settings.selected_model.clone();
    let old_onboarding_completed = settings.onboarding_completed;
    let old_language = settings.selected_language.clone();
    let old_language_from_hint = settings.language_from_hint;

    // Persist the new selection early so the frontend sees the correct model
    // when it reacts to events emitted by load_model.
    let mut settings = settings;
    settings.selected_model = model_id.to_string();
    settings.onboarding_completed = true;
    apply_language_hint(&mut settings, model_id);

    write_settings(app, settings);

    // Skip eager loading if unload is set to "Immediately" — the model
    // will be loaded on-demand during the next transcription.
    if unload_timeout == ModelUnloadTimeout::Immediately {
        // Notify frontend — load_model won't be called so no events
        // would otherwise be emitted.
        let _ = app.emit(
            "model-state-changed",
            ModelStateEvent {
                event_type: "selection_changed".to_string(),
                model_id: Some(model_id.to_string()),
                model_name: Some(model_info.name.clone()),
                error: None,
            },
        );
        log::info!(
            "Model selection changed to {} (not loading — unload set to Immediately).",
            model_id
        );
        return Ok(());
    }

    // Load the model. On failure, revert the persisted selection.
    if let Err(e) = transcription_manager.load_model(model_id) {
        let mut settings = get_settings(app);
        settings.selected_model = old_model;
        settings.onboarding_completed = old_onboarding_completed;
        settings.selected_language = old_language;
        settings.language_from_hint = old_language_from_hint;
        write_settings(app, settings);
        return Err(e.to_string());
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_active_model(
    app_handle: AppHandle,
    _model_manager: State<'_, Arc<ModelManager>>,
    _transcription_manager: State<'_, Arc<TranscriptionManager>>,
    model_id: String,
) -> Result<(), String> {
    switch_active_model(&app_handle, &model_id)
}

#[tauri::command]
#[specta::specta]
pub async fn get_current_model(app_handle: AppHandle) -> Result<String, String> {
    let settings = get_settings(&app_handle);
    Ok(settings.selected_model)
}

#[tauri::command]
#[specta::specta]
pub async fn get_transcription_model_status(
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
) -> Result<Option<String>, String> {
    Ok(transcription_manager.get_current_model())
}

#[tauri::command]
#[specta::specta]
pub async fn is_model_loading(
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
) -> Result<bool, String> {
    // Check if transcription manager has a loaded model
    let current_model = transcription_manager.get_current_model();
    Ok(current_model.is_none())
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_download(
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
) -> Result<(), String> {
    model_manager
        .cancel_download(&model_id)
        .map_err(|e| e.to_string())
}

fn is_user_defined(model_id: &str) -> bool {
    model_id.starts_with(custom_models::ID_PREFIX)
}

/// Apply the language hint of the newly selected model. A hint only replaces
/// "auto" or a language set by an earlier hint; a language the user picked
/// is left alone. Switching to a model without a hint undoes a hint-set
/// language.
fn apply_language_hint(settings: &mut AppSettings, model_id: &str) {
    let hint = settings
        .custom_models
        .iter()
        .find(|d| d.id == model_id)
        .and_then(|d| d.language_hint.clone());
    let replaceable = settings.selected_language == "auto" || settings.language_from_hint;
    match hint {
        Some(language) if replaceable => {
            settings.selected_language = language;
            settings.language_from_hint = true;
        }
        None if settings.language_from_hint => {
            settings.selected_language = "auto".to_string();
            settings.language_from_hint = false;
        }
        _ => {}
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_hf_repo_model_files(repo_id: String) -> Result<HfRepoListing, String> {
    custom_models::list_hf_repo_files(&repo_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn add_custom_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    model: NewCustomModel,
) -> Result<CustomModelDef, String> {
    let mut settings = get_settings(&app_handle);
    let def = custom_models::build_definition(model, &settings.custom_models)
        .map_err(|e| e.to_string())?;
    if model_manager.get_model_info(&def.id).is_some() {
        return Err("A model with this id already exists".to_string());
    }
    settings.custom_models.push(def.clone());
    write_settings(&app_handle, settings);
    model_manager
        .register_user_defined_model(&def)
        .map_err(|e| e.to_string())?;
    log::info!("Added custom model {}", def.id);
    Ok(def)
}

#[tauri::command]
#[specta::specta]
pub async fn update_custom_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    model_id: String,
    name: String,
    language_hint: Option<String>,
) -> Result<CustomModelDef, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Name is required".to_string());
    }
    let language_hint = language_hint
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty() && h != "auto");

    let mut settings = get_settings(&app_handle);
    let def = settings
        .custom_models
        .iter_mut()
        .find(|d| d.id == model_id)
        .ok_or_else(|| format!("Custom model not found: {model_id}"))?;
    def.name = name;
    def.language_hint = language_hint;
    let def = def.clone();

    // Re-apply the hint if this model is the active one.
    if settings.selected_model == model_id {
        apply_language_hint(&mut settings, &model_id);
    }
    write_settings(&app_handle, settings);
    model_manager
        .register_user_defined_model(&def)
        .map_err(|e| e.to_string())?;
    Ok(def)
}

#[tauri::command]
#[specta::specta]
pub async fn remove_custom_model(
    app_handle: AppHandle,
    model_manager: State<'_, Arc<ModelManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    model_id: String,
    delete_file: bool,
) -> Result<(), String> {
    let mut settings = get_settings(&app_handle);
    if !settings.custom_models.iter().any(|d| d.id == model_id) {
        return Err(format!("Custom model not found: {model_id}"));
    }
    if settings.selected_model == model_id {
        transcription_manager
            .unload_model()
            .map_err(|e| format!("Failed to unload model: {e}"))?;
        settings.selected_model = String::new();
        if settings.language_from_hint {
            settings.selected_language = "auto".to_string();
            settings.language_from_hint = false;
        }
    }
    settings.custom_models.retain(|d| d.id != model_id);
    write_settings(&app_handle, settings);
    model_manager
        .unregister_user_defined_model(&model_id, delete_file)
        .map_err(|e| e.to_string())?;
    log::info!("Removed custom model {model_id} (delete_file={delete_file})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{get_default_settings, CustomModelSource};

    fn with_model(hint: Option<&str>) -> AppSettings {
        let mut s = get_default_settings();
        s.custom_models.push(CustomModelDef {
            id: "custom:a/b/m.bin".into(),
            name: "M".into(),
            source: CustomModelSource::HuggingFace {
                repo_id: "a/b".into(),
                revision: "0".repeat(40),
                filename: "m.bin".into(),
            },
            language_hint: hint.map(String::from),
            size_bytes: None,
        });
        s
    }

    #[test]
    fn hint_replaces_auto() {
        let mut s = with_model(Some("no"));
        apply_language_hint(&mut s, "custom:a/b/m.bin");
        assert_eq!(s.selected_language, "no");
        assert!(s.language_from_hint);
    }

    #[test]
    fn hint_keeps_user_choice() {
        let mut s = with_model(Some("no"));
        s.selected_language = "en".into();
        apply_language_hint(&mut s, "custom:a/b/m.bin");
        assert_eq!(s.selected_language, "en");
        assert!(!s.language_from_hint);
    }

    #[test]
    fn switching_away_undoes_hint() {
        let mut s = with_model(Some("no"));
        apply_language_hint(&mut s, "custom:a/b/m.bin");
        apply_language_hint(&mut s, "parakeet-tdt-0.6b-v3");
        assert_eq!(s.selected_language, "auto");
        assert!(!s.language_from_hint);
    }

    #[test]
    fn no_hint_leaves_user_choice() {
        let mut s = with_model(None);
        s.selected_language = "de".into();
        apply_language_hint(&mut s, "custom:a/b/m.bin");
        assert_eq!(s.selected_language, "de");
    }
}
