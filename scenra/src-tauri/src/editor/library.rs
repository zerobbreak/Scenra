//! Reading recorded sessions from disk for the editor: which scenes and takes exist, how long
//! each take is, its click log, and the session's edit (or a sensible default).

use super::model::{Edit, SceneEdit, TakeFacts, Timeline, timeline};
use crate::ai::Scenario;
use crate::session::{self, SessionInfo};
use crate::take::{ACTIONS_FILE, ActionRecord};
use serde::Serialize;
use super::transcript::{self, Word};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const EDIT_FILE: &str = "edit.json";
pub const EXPORT_DIR: &str = "export";
pub const VIDEO: &str = "video.mp4";
pub const MIC: &str = "microphone.wav";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TakeInfo {
    pub folder: String,
    pub scene: usize,
    pub attempt: u32,
    pub duration: f64,
    /// Recorded before click logging existed: cuts can't be checked against clicks.
    pub has_actions: bool,
    pub actions: Vec<ActionRecord>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub dir: String,
    pub title: String,
    pub scenes: usize,
    pub takes: usize,
    pub exported: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionData {
    pub id: String,
    pub dir: String,
    pub title: String,
    pub script: Option<Scenario>,
    pub scene_count: usize,
    pub takes: Vec<TakeInfo>,
    pub edit: Edit,
    pub timeline: Timeline,
    pub exports: Vec<String>,
    /// Cached transcripts by take folder (only those still matching their recording).
    pub transcripts: BTreeMap<String, Vec<Word>>,
}

/// `scene-02-take-3` → (1, 3).
pub fn parse_folder(name: &str) -> Option<(usize, u32)> {
    let rest = name.strip_prefix("scene-")?;
    let (scene, attempt) = rest.split_once("-take-")?;
    let scene: usize = scene.parse().ok()?;
    Some((scene.checked_sub(1)?, attempt.parse().ok()?))
}

/// Length of a PCM WAV in seconds, from its `fmt ` and `data` chunks.
pub fn wav_seconds(path: &Path) -> Option<f64> {
    use std::io::Read;
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let head = &head[..n];
    if head.get(..4)? != b"RIFF" || head.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut at, mut byte_rate, mut data) = (12usize, None, None);
    while at + 8 <= head.len() {
        let id = &head[at..at + 4];
        let size = u32::from_le_bytes(head[at + 4..at + 8].try_into().ok()?) as usize;
        if id == b"fmt " {
            byte_rate = Some(u32::from_le_bytes(head.get(at + 16..at + 20)?.try_into().ok()?));
        } else if id == b"data" {
            data = Some(size);
            break;
        }
        at += 8 + size + (size & 1);
    }
    Some(data? as f64 / f64::from(byte_rate.filter(|r| *r > 0)?))
}

/// Length of an MP4 in seconds, from the `mvhd` box inside `moov` (Windows writes `moov` last,
/// so top-level boxes are walked by seeking).
pub fn mp4_seconds(path: &Path) -> Option<f64> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut at = 0u64;
    while at + 8 <= len {
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head[..8]).ok()?;
        let mut size = u64::from(u32::from_be_bytes(head[..4].try_into().ok()?));
        let mut header = 8;
        if size == 1 {
            file.read_exact(&mut head[8..16]).ok()?;
            size = u64::from_be_bytes(head[8..16].try_into().ok()?);
            header = 16;
        } else if size == 0 {
            size = len - at;
        }
        if &head[4..8] == b"moov" {
            let mut moov = vec![0u8; usize::try_from(size.checked_sub(header)?.min(64 << 20)).ok()?];
            file.read_exact(&mut moov).ok()?;
            let mut i = 0usize;
            while i + 8 <= moov.len() {
                let box_size = u32::from_be_bytes(moov[i..i + 4].try_into().ok()?) as usize;
                if &moov[i + 4..i + 8] == b"mvhd" {
                    let b = &moov[i + 8..];
                    let (scale, duration) = if b.first()? == &1 {
                        (u32::from_be_bytes(b.get(20..24)?.try_into().ok()?), u64::from_be_bytes(b.get(24..32)?.try_into().ok()?))
                    } else {
                        (u32::from_be_bytes(b.get(12..16)?.try_into().ok()?), u64::from(u32::from_be_bytes(b.get(16..20)?.try_into().ok()?)))
                    };
                    return (scale > 0).then(|| duration as f64 / f64::from(scale));
                }
                if box_size < 8 {
                    return None;
                }
                i += box_size;
            }
            return None;
        }
        if size < 8 {
            return None;
        }
        at += size;
    }
    None
}

/// The highest attempt number per scene among every take folder, playable or not, so a retake
/// never reuses a folder that already holds files.
pub fn attempts_in(dir: &Path) -> std::collections::BTreeMap<usize, u32> {
    let mut attempts = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if let Some((scene, attempt)) = parse_folder(&entry.file_name().to_string_lossy()) {
            let n = attempts.entry(scene).or_insert(0);
            *n = (*n).max(attempt);
        }
    }
    attempts
}

fn takes_in(dir: &Path) -> Vec<TakeInfo> {
    let mut takes: Vec<TakeInfo> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let folder = entry.file_name().to_string_lossy().into_owned();
            let (scene, attempt) = parse_folder(&folder)?;
            let path = entry.path();
            if !path.join(VIDEO).is_file() {
                return None; // recorded with OBS (video.mkv) or unfinished
            }
            // The video ends at its last captured frame, the mic when recording stopped: use the
            // shorter, so nothing asks for video that isn't there.
            let audio = wav_seconds(&path.join(MIC))?;
            let duration = mp4_seconds(&path.join(VIDEO)).map_or(audio, |video| video.min(audio));
            let actions: Option<Vec<ActionRecord>> = session::read_json(&path.join(ACTIONS_FILE));
            Some(TakeInfo { folder, scene, attempt, duration, has_actions: actions.is_some(), actions: actions.unwrap_or_default() })
        })
        .collect();
    takes.sort_by_key(|t| (t.scene, t.attempt));
    takes
}

fn exports_in(dir: &Path) -> Vec<String> {
    let mut files: Vec<String> = std::fs::read_dir(dir.join(EXPORT_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4")))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    files.sort();
    files
}

/// Every session in `dirs`, newest first. A session is a folder with at least one usable take.
pub fn list(dirs: &[PathBuf]) -> Vec<SessionSummary> {
    let mut out: Vec<SessionSummary> = dirs
        .iter()
        .flat_map(|root| std::fs::read_dir(root).into_iter().flatten().flatten())
        .filter(|e| e.path().is_dir() && e.file_name() != session::CHECKS)
        .filter_map(|e| {
            let dir = e.path();
            let takes = takes_in(&dir);
            if takes.is_empty() {
                return None;
            }
            let id = e.file_name().to_string_lossy().into_owned();
            let info: Option<SessionInfo> = session::read_json(&dir.join(session::SESSION_FILE));
            let scenes: BTreeSet<usize> = takes.iter().map(|t| t.scene).collect();
            Some(SessionSummary {
                title: info.map(|i| i.title).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("Session {id}")),
                id,
                scenes: scenes.len(),
                takes: takes.len(),
                exported: !exports_in(&dir).is_empty(),
                dir: dir.to_string_lossy().into_owned(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

/// The default edit: every scene on its latest take, untrimmed.
fn default_edit(scene_count: usize, takes: &[TakeInfo]) -> Edit {
    let scenes = (0..scene_count)
        .map(|scene| SceneEdit {
            scene,
            take: takes.iter().filter(|t| t.scene == scene).max_by_key(|t| t.attempt).map(|t| t.folder.clone()),
            ..Default::default()
        })
        .collect();
    Edit { scenes, ..Default::default() }
}

/// Makes sure every scene has an entry, keeping what the presenter already chose.
fn reconcile(mut edit: Edit, scene_count: usize, takes: &[TakeInfo]) -> Edit {
    let defaults = default_edit(scene_count, takes);
    for scene in defaults.scenes {
        if !edit.scenes.iter().any(|s| s.scene == scene.scene) {
            edit.scenes.push(scene);
        }
    }
    edit.scenes.sort_by_key(|s| s.scene);
    edit
}

pub fn timeline_of(edit: &Edit, takes: &[TakeInfo]) -> Timeline {
    timeline(edit, |scene: &SceneEdit| {
        let take = takes.iter().find(|t| Some(&t.folder) == scene.take.as_ref() && t.scene == scene.scene)?;
        Some(TakeFacts { duration: take.duration, actions: &take.actions })
    })
}

pub fn load(dir: &Path) -> Result<SessionData, String> {
    if !dir.is_dir() {
        return Err(format!("{} doesn't exist", dir.display()));
    }
    let id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let info: Option<SessionInfo> = session::read_json(&dir.join(session::SESSION_FILE));
    let script: Option<Scenario> = session::read_json(&dir.join(session::SCRIPT_FILE));
    let takes = takes_in(dir);
    let scene_count = script
        .as_ref()
        .map(|s| s.scenes.len())
        .unwrap_or(0)
        .max(takes.iter().map(|t| t.scene + 1).max().unwrap_or(0));
    let edit = match session::read_json::<Edit>(&dir.join(EDIT_FILE)) {
        Some(saved) => reconcile(saved, scene_count, &takes),
        None => default_edit(scene_count, &takes),
    };
    let timeline = timeline_of(&edit, &takes);
    let transcripts = takes
        .iter()
        .filter_map(|t| {
            let take_dir = dir.join(&t.folder);
            transcript::cached(&take_dir, &take_dir.join(MIC)).map(|tr| (t.folder.clone(), tr.words))
        })
        .collect();
    Ok(SessionData {
        transcripts,
        title: info.map(|i| i.title).filter(|t| !t.is_empty()).or_else(|| script.as_ref().map(|s| s.title.clone())).unwrap_or_else(|| format!("Session {id}")),
        id,
        dir: dir.to_string_lossy().into_owned(),
        script,
        scene_count,
        takes,
        edit,
        timeline,
        exports: exports_in(dir),
    })
}

pub fn save_edit(dir: &Path, edit: &Edit) -> Result<(), String> {
    session::write_json(&dir.join(EDIT_FILE), edit)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::recorder::wav_header as header;

    pub(crate) fn make_take(session: &Path, folder: &str, seconds: f64) {
        let dir = session.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(VIDEO), fake_mp4(seconds + 1.0)).unwrap();
        let bytes = (seconds * 48_000.0) as u32 * 2;
        let mut wav = header(48_000, 1, bytes).to_vec();
        wav.resize(44 + bytes as usize, 0);
        std::fs::write(dir.join(MIC), wav).unwrap();
    }

    /// An MP4 with just enough boxes for `mp4_seconds`: ftyp, mdat, then moov/mvhd (v0).
    pub(crate) fn fake_mp4(seconds: f64) -> Vec<u8> {
        let mut mvhd = vec![0u8; 100];
        mvhd[12..16].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&((seconds * 1000.0) as u32).to_be_bytes());
        let boxed = |kind: &[u8], body: &[u8]| [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat();
        [boxed(b"ftyp", b"isom0000"), boxed(b"mdat", &[0; 32]), boxed(b"moov", &boxed(b"mvhd", &mvhd))].concat()
    }

    #[test]
    fn take_length_is_the_shorter_of_video_and_mic() {
        let root = temp("length");
        make_take(&root, "scene-01-take-1", 2.0);
        std::fs::write(root.join("scene-01-take-1").join(VIDEO), fake_mp4(1.75)).unwrap();
        assert_eq!(mp4_seconds(&root.join("scene-01-take-1").join(VIDEO)), Some(1.75));
        assert_eq!(takes_in(&root)[0].duration, 1.75);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scenra-lib-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn folder_names_give_scene_and_attempt() {
        assert_eq!(parse_folder("scene-02-take-3"), Some((1, 3)));
        assert_eq!(parse_folder("scene-00-take-1"), None);
        assert_eq!(parse_folder("export"), None);
    }

    #[test]
    fn default_edit_uses_the_latest_take_of_each_scene() {
        let root = temp("default");
        let s = root.join("20261031-100000");
        make_take(&s, "scene-01-take-1", 2.0);
        make_take(&s, "scene-01-take-2", 3.0);
        make_take(&s, "scene-02-take-1", 1.5);
        let data = load(&s).unwrap();
        assert_eq!(data.scene_count, 2);
        assert_eq!(data.edit.scenes[0].take.as_deref(), Some("scene-01-take-2"));
        assert_eq!(data.takes[1].duration, 3.0);
        assert_eq!(data.timeline.duration, 4.5);
        assert!(!data.takes[0].has_actions);

        let mut edit = data.edit.clone();
        edit.scenes[0].take = Some("scene-01-take-1".into());
        save_edit(&s, &edit).unwrap();
        assert_eq!(load(&s).unwrap().timeline.duration, 3.5);

        make_take(&root.join(session::CHECKS), "scene-01-take-1", 1.0);
        let listed = list(std::slice::from_ref(&root));
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].scenes, listed[0].takes), (2, 3));
        let _ = std::fs::remove_dir_all(&root);
    }
}
