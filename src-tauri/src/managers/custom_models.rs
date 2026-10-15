//! User-defined transcription models: a file from a public Hugging Face repo or
//! a direct HTTPS download. Definitions live in `AppSettings::custom_models`
//! and are registered into the model registry at startup and when added, so
//! the regular download / select / load / delete flow handles them.

use super::model::{EngineType, ModelInfo, ModelSource};
use crate::settings::{CustomModelDef, CustomModelSource};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

pub const ID_PREFIX: &str = "custom:";

const HF_API_BASE: &str = "https://huggingface.co/api/models";
const HF_API_TIMEOUT: Duration = Duration::from_secs(15);

/// GGUF files start with the ASCII magic `GGUF`.
const GGUF_MAGIC: [u8; 4] = *b"GGUF";
/// Legacy whisper.cpp GGML files start with 0x67676d6c stored little-endian.
const GGML_MAGIC: [u8; 4] = *b"lmgg";

#[derive(Debug, Clone, Serialize, Type)]
pub struct HfRepoFile {
    pub path: String,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct HfRepoListing {
    pub repo_id: String,
    /// Commit SHA the listing was taken from; used to pin the download.
    pub revision: String,
    pub files: Vec<HfRepoFile>,
}

/// Input from the "Add model" dialog. Ids and storage filenames are derived
/// on the backend.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(tag = "type")]
pub enum NewCustomModel {
    HuggingFace {
        name: String,
        repo_id: String,
        revision: String,
        filename: String,
        size_bytes: Option<u64>,
        language_hint: Option<String>,
    },
    Url {
        name: String,
        url: String,
        sha256: Option<String>,
        language_hint: Option<String>,
    },
}

pub fn is_supported_model_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".gguf") || lower.ends_with(".bin")
}

/// Files offered in the Hugging Face picker. `.bin` is also used by PyTorch,
/// CTranslate2 and training checkpoints, so only whisper.cpp-style `ggml*.bin`
/// files are listed.
fn is_listable_model_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    file.ends_with(".gguf") || (file.ends_with(".bin") && file.starts_with("ggml"))
}

fn is_valid_repo_id(repo_id: &str) -> bool {
    let mut parts = repo_id.split('/');
    let (Some(owner), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let valid_part = |p: &str| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    valid_part(owner) && valid_part(name)
}

fn is_valid_revision(revision: &str) -> bool {
    revision.len() == 40 && revision.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_valid_repo_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && path
            .split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn normalize_sha256(sha: Option<String>) -> Result<Option<String>> {
    match sha.map(|s| s.trim().to_ascii_lowercase()) {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) => Ok(Some(s)),
        Some(_) => bail!("SHA-256 must be 64 hexadecimal characters"),
    }
}

fn normalize_language_hint(hint: Option<String>) -> Option<String> {
    hint.map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty() && h != "auto")
}

/// Lowercase ASCII slug for ids and filenames: `NbAiLab/nb-whisper` → `nbailab-nb-whisper`.
fn slugify(input: &str) -> String {
    let mut slug = String::with_capacity(input.len());
    let mut last_dash = true;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn url_file_extension(url: &url::Url) -> Option<&'static str> {
    let last = url.path_segments()?.next_back()?.to_ascii_lowercase();
    if last.ends_with(".gguf") {
        Some("gguf")
    } else if last.ends_with(".bin") {
        Some("bin")
    } else {
        None
    }
}

/// Validate dialog input and turn it into a stored definition.
pub fn build_definition(
    input: NewCustomModel,
    existing: &[CustomModelDef],
) -> Result<CustomModelDef> {
    let def = match input {
        NewCustomModel::HuggingFace {
            name,
            repo_id,
            revision,
            filename,
            size_bytes,
            language_hint,
        } => {
            let repo_id = repo_id.trim().to_string();
            if !is_valid_repo_id(&repo_id) {
                bail!("Repository must look like 'owner/name'");
            }
            if !is_valid_revision(&revision) {
                bail!("Revision must be a full commit SHA");
            }
            if !is_valid_repo_path(&filename) || !is_supported_model_file(&filename) {
                bail!("Only .gguf and .bin model files are supported");
            }
            CustomModelDef {
                id: format!("{ID_PREFIX}{repo_id}/{filename}"),
                name: name.trim().to_string(),
                source: CustomModelSource::HuggingFace {
                    repo_id,
                    revision,
                    filename,
                },
                language_hint: normalize_language_hint(language_hint),
                size_bytes,
            }
        }
        NewCustomModel::Url {
            name,
            url,
            sha256,
            language_hint,
        } => {
            let parsed = url::Url::parse(url.trim()).map_err(|_| anyhow!("Not a valid URL"))?;
            if parsed.scheme() != "https" {
                bail!("Only https:// URLs are supported");
            }
            if parsed.host_str().is_none() {
                bail!("URL has no host");
            }
            let ext = url_file_extension(&parsed)
                .ok_or_else(|| anyhow!("URL must point to a .gguf or .bin file"))?;
            let name = name.trim().to_string();
            let mut slug = slugify(&name);
            if slug.is_empty() {
                slug = "model".to_string();
            }
            // Unique, Handy-owned filename so URL models never collide with
            // catalog files or with each other.
            let mut candidate = slug.clone();
            let mut n = 2;
            while existing
                .iter()
                .any(|d| d.id == format!("{ID_PREFIX}url/{candidate}"))
            {
                candidate = format!("{slug}-{n}");
                n += 1;
            }
            CustomModelDef {
                id: format!("{ID_PREFIX}url/{candidate}"),
                name,
                source: CustomModelSource::Url {
                    url: parsed.to_string(),
                    sha256: normalize_sha256(sha256)?,
                    filename: format!("custom-{candidate}.{ext}"),
                },
                language_hint: normalize_language_hint(language_hint),
                size_bytes: None,
            }
        }
    };

    if def.name.is_empty() {
        bail!("Name is required");
    }
    if existing.iter().any(|d| d.id == def.id) {
        bail!("This model has already been added");
    }
    Ok(def)
}

pub fn to_model_info(def: &CustomModelDef) -> ModelInfo {
    let (source, filename, origin) = match &def.source {
        CustomModelSource::HuggingFace {
            repo_id,
            revision,
            filename,
        } => (
            ModelSource::HuggingFace {
                repo_id: repo_id.clone(),
                revision: revision.clone(),
            },
            filename.clone(),
            format!("Hugging Face: {repo_id}/{filename}"),
        ),
        CustomModelSource::Url {
            url,
            sha256,
            filename,
        } => (
            ModelSource::Url {
                url: url.clone(),
                sha256: sha256.clone(),
            },
            filename.clone(),
            match url::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
            {
                Some(host) => format!("Download from {host}"),
                None => "Direct download".to_string(),
            },
        ),
    };

    ModelInfo {
        id: def.id.clone(),
        name: def.name.clone(),
        description: format!("Custom model · {origin}"),
        filename,
        source,
        size_mb: def.size_bytes.map(|b| b / (1024 * 1024)).unwrap_or(0),
        is_downloaded: false,
        is_downloading: false,
        partial_size: 0,
        is_directory: false,
        engine_type: EngineType::TranscribeCpp,
        accuracy_score: 0.0,
        speed_score: 0.0,
        supports_translation: false,
        is_recommended: false,
        // Unknown until transcribe-cpp reports capabilities at load; an empty
        // list passes the selected language straight through to the engine.
        supported_languages: Vec::new(),
        supports_language_selection: false,
        // Not "custom" in the drop-in sense: the entry is persistent and can
        // be (re-)downloaded, so it must not vanish when its file is missing.
        is_custom: false,
        supports_streaming: false,
        supports_language_detection: false,
    }
}

/// Check the first bytes of a downloaded file against the formats
/// transcribe-cpp can load.
pub fn verify_model_file(path: &Path) -> Result<()> {
    let mut magic = [0u8; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map_err(|e| anyhow!("Could not read model file: {e}"))?;
    if magic == GGUF_MAGIC || magic == GGML_MAGIC {
        Ok(())
    } else {
        bail!("The downloaded file is not a GGUF or whisper.cpp GGML model")
    }
}

#[derive(Deserialize)]
struct HfModelResponse {
    sha: String,
    #[serde(default)]
    siblings: Vec<HfSibling>,
}

#[derive(Deserialize)]
struct HfSibling {
    rfilename: String,
    #[serde(default)]
    size: Option<u64>,
}

fn listing_from_response(repo_id: &str, body: HfModelResponse) -> Result<HfRepoListing> {
    if !is_valid_revision(&body.sha) {
        bail!("Hugging Face returned an unexpected revision");
    }
    let mut files: Vec<HfRepoFile> = body
        .siblings
        .into_iter()
        .filter(|s| is_listable_model_file(&s.rfilename) && is_valid_repo_path(&s.rfilename))
        .map(|s| HfRepoFile {
            path: s.rfilename,
            size_bytes: s.size,
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(HfRepoListing {
        repo_id: repo_id.to_string(),
        revision: body.sha,
        files,
    })
}

/// List the `.gguf` / `.bin` files of a public Hugging Face model repo at its
/// current `main` commit.
pub async fn list_hf_repo_files(repo_id: &str) -> Result<HfRepoListing> {
    let repo_id = repo_id.trim();
    if !is_valid_repo_id(repo_id) {
        bail!("Repository must look like 'owner/name'");
    }
    let client = reqwest::Client::builder()
        .timeout(HF_API_TIMEOUT)
        .user_agent(concat!("Handy/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client
        .get(format!("{HF_API_BASE}/{repo_id}/revision/main"))
        .query(&[("blobs", "true")])
        .send()
        .await
        .map_err(|e| anyhow!("Could not reach Hugging Face: {e}"))?;
    match response.status() {
        s if s.is_success() => {}
        reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::UNAUTHORIZED => {
            bail!("Repository not found, or it is private")
        }
        s => bail!("Hugging Face returned HTTP {s}"),
    }
    let body: HfModelResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("Unexpected response from Hugging Face: {e}"))?;
    listing_from_response(repo_id, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    fn hf_input(filename: &str) -> NewCustomModel {
        NewCustomModel::HuggingFace {
            name: "NB Whisper Large".into(),
            repo_id: "NbAiLab/nb-whisper-large".into(),
            revision: SHA.into(),
            filename: filename.into(),
            size_bytes: Some(1_081_140_203),
            language_hint: Some("no".into()),
        }
    }

    fn url_input(name: &str, url: &str) -> NewCustomModel {
        NewCustomModel::Url {
            name: name.into(),
            url: url.into(),
            sha256: None,
            language_hint: None,
        }
    }

    #[test]
    fn builds_hf_definition() {
        let def = build_definition(hf_input("ggml-model-q5_0.bin"), &[]).unwrap();
        assert_eq!(
            def.id,
            "custom:NbAiLab/nb-whisper-large/ggml-model-q5_0.bin"
        );
        assert_eq!(def.language_hint.as_deref(), Some("no"));
        let info = to_model_info(&def);
        assert_eq!(info.filename, "ggml-model-q5_0.bin");
        assert_eq!(info.size_mb, 1031);
        assert!(!info.is_custom);
        assert!(matches!(info.source, ModelSource::HuggingFace { .. }));
    }

    #[test]
    fn rejects_bad_hf_input() {
        assert!(build_definition(hf_input("model.safetensors"), &[]).is_err());
        assert!(build_definition(hf_input("../evil.bin"), &[]).is_err());
        let bad_repo = NewCustomModel::HuggingFace {
            name: "x".into(),
            repo_id: "no-slash".into(),
            revision: SHA.into(),
            filename: "a.bin".into(),
            size_bytes: None,
            language_hint: None,
        };
        assert!(build_definition(bad_repo, &[]).is_err());
        let bad_rev = NewCustomModel::HuggingFace {
            name: "x".into(),
            repo_id: "a/b".into(),
            revision: "main".into(),
            filename: "a.bin".into(),
            size_bytes: None,
            language_hint: None,
        };
        assert!(build_definition(bad_rev, &[]).is_err());
    }

    #[test]
    fn rejects_duplicates() {
        let first = build_definition(hf_input("ggml-model-q5_0.bin"), &[]).unwrap();
        assert!(build_definition(hf_input("ggml-model-q5_0.bin"), &[first]).is_err());
    }

    #[test]
    fn url_models_get_unique_filenames() {
        let a = build_definition(
            url_input("My Model", "https://example.com/x/model.gguf"),
            &[],
        )
        .unwrap();
        assert_eq!(a.id, "custom:url/my-model");
        match &a.source {
            CustomModelSource::Url { filename, .. } => {
                assert_eq!(filename, "custom-my-model.gguf")
            }
            _ => panic!("expected url source"),
        }
        let b = build_definition(url_input("My Model", "https://example.com/y.bin"), &[a]).unwrap();
        assert_eq!(b.id, "custom:url/my-model-2");
    }

    #[test]
    fn rejects_bad_urls() {
        assert!(build_definition(url_input("a", "http://example.com/m.bin"), &[]).is_err());
        assert!(build_definition(url_input("a", "https://example.com/m.onnx"), &[]).is_err());
        assert!(build_definition(url_input("a", "not a url"), &[]).is_err());
        let bad_sha = NewCustomModel::Url {
            name: "a".into(),
            url: "https://example.com/m.bin".into(),
            sha256: Some("abc".into()),
            language_hint: None,
        };
        assert!(build_definition(bad_sha, &[]).is_err());
        assert!(build_definition(url_input("  ", "https://example.com/m.bin"), &[]).is_err());
    }

    #[test]
    fn auto_language_hint_is_dropped() {
        let input = NewCustomModel::Url {
            name: "a".into(),
            url: "https://example.com/m.bin".into(),
            sha256: None,
            language_hint: Some("auto".into()),
        };
        assert_eq!(build_definition(input, &[]).unwrap().language_hint, None);
    }

    #[test]
    fn verifies_magic_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let check = |bytes: &[u8]| {
            let path = dir.path().join("m.bin");
            File::create(&path).unwrap().write_all(bytes).unwrap();
            verify_model_file(&path)
        };
        assert!(check(b"GGUF\x03\0\0\0").is_ok());
        assert!(check(b"lmgg\x00\x00").is_ok());
        assert!(check(b"<html>").is_err());
        assert!(check(b"GG").is_err());
    }

    #[test]
    fn filters_hf_listing() {
        let body: HfModelResponse = serde_json::from_value(serde_json::json!({
            "sha": SHA,
            "siblings": [
                {"rfilename": "ggml-model.bin", "size": 3095033483u64},
                {"rfilename": "ggml-model-q5_0.bin", "size": 1081140203u64},
                {"rfilename": "pytorch_model.bin", "size": 6174380865u64},
                {"rfilename": "ct2/model.bin", "size": 6174511117u64},
                {"rfilename": "training_state.bin", "size": 6331},
                {"rfilename": "model.safetensors", "size": 1},
                {"rfilename": "onnx/encoder.onnx"},
                {"rfilename": "sub/q8.GGUF"}
            ]
        }))
        .unwrap();
        let listing = listing_from_response("a/b", body).unwrap();
        let paths: Vec<_> = listing.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["ggml-model-q5_0.bin", "ggml-model.bin", "sub/q8.GGUF"]
        );
        assert_eq!(listing.revision, SHA);
    }
}
