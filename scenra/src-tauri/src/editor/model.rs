//! The editor's non-destructive edit (`edit.json`) and the one function every other part uses:
//! `timeline`, which turns the edit into the ordered source segments that make the finished video.
//! Preview plays these segments, export renders them, and captions are timed through them.
//!
//! Times are seconds from a take's first video frame (the mic is aligned to it by the recorder).

use crate::take::ActionRecord;
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
/// Pieces shorter than this after cutting are dropped: a few frames of video read as a glitch.
const MIN_SEGMENT: f64 = 0.05;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Source {
    Manual,
    /// Made by Tidy up or Assemble with AI; the reason is shown next to the change.
    Ai { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cut {
    pub from: f64,
    pub to: f64,
    pub source: Source,
    /// Switched off instead of deleted, so an AI cut can be restored.
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SceneEdit {
    /// 0-based scene index.
    pub scene: usize,
    /// Take folder name (`scene-02-take-3`); `None` while the scene has no take.
    pub take: Option<String>,
    /// Why Assemble with AI chose this take; `None` when the presenter chose it.
    pub take_note: Option<String>,
    pub trim_in: f64,
    /// `None` = to the end of the take.
    pub trim_out: Option<f64>,
    /// Why Tidy up set the trims (shown with an undo); `None` when the presenter set them.
    pub trim_note: Option<String>,
    pub cuts: Vec<Cut>,
    pub excluded: bool,
}

impl Default for SceneEdit {
    fn default() -> Self {
        Self { scene: 0, take: None, take_note: None, trim_in: 0.0, trim_out: None, trim_note: None, cuts: Vec::new(), excluded: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Edit {
    pub version: u32,
    pub scenes: Vec<SceneEdit>,
}

impl Default for Edit {
    fn default() -> Self {
        Self { version: VERSION, scenes: Vec::new() }
    }
}

/// What `timeline` needs to know about the chosen take of a scene.
pub struct TakeFacts<'a> {
    pub duration: f64,
    pub actions: &'a [ActionRecord],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub scene: usize,
    pub take: String,
    /// Source range in the take, seconds.
    pub src_in: f64,
    pub src_out: f64,
    /// Where it starts in the finished video, seconds.
    pub out_at: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    pub segments: Vec<Segment>,
    pub duration: f64,
    pub warnings: Vec<String>,
}

fn round(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

/// The kept source ranges of one scene: [trim_in, trim_out] minus the enabled cuts that don't
/// touch a click or the cursor's approach to it.
fn kept_ranges(scene: &SceneEdit, facts: &TakeFacts, warnings: &mut Vec<String>) -> Vec<(f64, f64)> {
    let lo = scene.trim_in.clamp(0.0, facts.duration);
    let hi = scene.trim_out.unwrap_or(facts.duration).clamp(lo, facts.duration);
    let mut cuts: Vec<(f64, f64)> = Vec::new();
    for cut in scene.cuts.iter().filter(|c| c.enabled && c.to > c.from) {
        if let Some(action) = facts.actions.iter().find(|a| cut.from < a.end && cut.to > a.start) {
            warnings.push(format!(
                "Scene {}: the cut at {:.1} s was skipped because it would cut through the {} on “{}”",
                scene.scene + 1,
                cut.from,
                if action.kind == "press" { "click" } else { "cursor moving" },
                action.label
            ));
            continue;
        }
        cuts.push((cut.from, cut.to));
    }
    cuts.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut ranges = Vec::new();
    let mut at = lo;
    for (from, to) in cuts {
        if to <= at {
            continue;
        }
        if from > at {
            ranges.push((at, from.min(hi)));
        }
        at = at.max(to);
        if at >= hi {
            break;
        }
    }
    if at < hi {
        ranges.push((at, hi));
    }
    ranges.retain(|(a, b)| b - a >= MIN_SEGMENT);
    ranges
}

/// The finished video as source segments, in scene order. `facts(scene)` describes the chosen
/// take of a scene, or `None` if it can't be used (missing files).
pub fn timeline<'a>(edit: &Edit, facts: impl Fn(&SceneEdit) -> Option<TakeFacts<'a>>) -> Timeline {
    let mut out = Timeline::default();
    let mut scenes: Vec<&SceneEdit> = edit.scenes.iter().collect();
    scenes.sort_by_key(|s| s.scene);
    for scene in scenes {
        if scene.excluded {
            continue;
        }
        let Some(take) = &scene.take else {
            out.warnings.push(format!("Scene {} has no take yet, so it's left out", scene.scene + 1));
            continue;
        };
        let Some(facts) = facts(scene) else {
            out.warnings.push(format!("Scene {}: {take} can't be read, so it's left out", scene.scene + 1));
            continue;
        };
        for (src_in, src_out) in kept_ranges(scene, &facts, &mut out.warnings) {
            out.segments.push(Segment {
                scene: scene.scene,
                take: take.clone(),
                src_in: round(src_in),
                src_out: round(src_out),
                out_at: round(out.duration),
            });
            out.duration += src_out - src_in;
        }
    }
    out.duration = round(out.duration);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(n: usize, take: &str) -> SceneEdit {
        SceneEdit { scene: n, take: Some(take.into()), ..Default::default() }
    }

    fn cut(from: f64, to: f64) -> Cut {
        Cut { from, to, source: Source::Manual, enabled: true }
    }

    fn click(start: f64, end: f64) -> ActionRecord {
        ActionRecord { kind: "press".into(), label: "Settings".into(), start, end, ok: true }
    }

    fn run(edit: &Edit, actions: &[ActionRecord]) -> Timeline {
        timeline(edit, |_| Some(TakeFacts { duration: 10.0, actions }))
    }

    fn ranges(t: &Timeline) -> Vec<(f64, f64, f64)> {
        t.segments.iter().map(|s| (s.src_in, s.src_out, s.out_at)).collect()
    }

    #[test]
    fn scenes_play_in_order_back_to_back() {
        let edit = Edit { scenes: vec![scene(1, "scene-02-take-1"), scene(0, "scene-01-take-2")], ..Default::default() };
        let t = run(&edit, &[]);
        assert_eq!(t.segments[0].take, "scene-01-take-2");
        assert_eq!(ranges(&t), vec![(0.0, 10.0, 0.0), (0.0, 10.0, 10.0)]);
        assert_eq!(t.duration, 20.0);
    }

    #[test]
    fn trims_and_cuts_remove_source_time() {
        let mut s = scene(0, "t");
        s.trim_in = 1.0;
        s.trim_out = Some(9.0);
        s.cuts = vec![cut(3.0, 4.5), cut(6.0, 7.0)];
        let t = run(&Edit { scenes: vec![s], ..Default::default() }, &[]);
        assert_eq!(ranges(&t), vec![(1.0, 3.0, 0.0), (4.5, 6.0, 2.0), (7.0, 9.0, 3.5)]);
        assert_eq!(t.duration, 5.5);
    }

    #[test]
    fn overlapping_and_outside_cuts_are_merged_and_clamped() {
        let mut s = scene(0, "t");
        s.cuts = vec![cut(2.0, 4.0), cut(3.0, 5.0), cut(-1.0, 0.5), cut(9.5, 12.0)];
        let t = run(&Edit { scenes: vec![s], ..Default::default() }, &[]);
        assert_eq!(ranges(&t), vec![(0.5, 2.0, 0.0), (5.0, 9.5, 1.5)]);
    }

    #[test]
    fn a_cut_through_a_click_is_skipped_with_a_warning() {
        let mut s = scene(0, "t");
        s.cuts = vec![cut(4.0, 6.0), cut(8.0, 8.5)];
        let t = run(&Edit { scenes: vec![s], ..Default::default() }, &[click(5.5, 6.2)]);
        assert_eq!(ranges(&t), vec![(0.0, 8.0, 0.0), (8.5, 10.0, 8.0)]);
        assert_eq!(t.warnings.len(), 1);
        assert!(t.warnings[0].contains("click on “Settings”"), "{}", t.warnings[0]);
    }

    #[test]
    fn disabled_cuts_and_excluded_scenes_are_ignored() {
        let mut a = scene(0, "a");
        a.cuts = vec![Cut { enabled: false, ..cut(1.0, 2.0) }];
        let mut b = scene(1, "b");
        b.excluded = true;
        let t = run(&Edit { scenes: vec![a, b], ..Default::default() }, &[]);
        assert_eq!(ranges(&t), vec![(0.0, 10.0, 0.0)]);
    }

    #[test]
    fn slivers_between_cuts_are_dropped() {
        let mut s = scene(0, "t");
        s.cuts = vec![cut(2.0, 4.0), cut(4.02, 6.0)];
        let t = run(&Edit { scenes: vec![s], ..Default::default() }, &[]);
        assert_eq!(ranges(&t), vec![(0.0, 2.0, 0.0), (6.0, 10.0, 2.0)]);
    }

    #[test]
    fn missing_takes_are_reported() {
        let edit = Edit { scenes: vec![SceneEdit { scene: 0, ..Default::default() }, scene(1, "gone")], ..Default::default() };
        let t = timeline(&edit, |_| None);
        assert!(t.segments.is_empty());
        assert_eq!(t.warnings.len(), 2);
    }

    #[test]
    fn edit_json_from_an_older_version_still_loads() {
        let e: Edit = serde_json::from_str(r#"{ "scenes": [{ "scene": 0, "take": "scene-01-take-1" }] }"#).unwrap();
        assert_eq!(e.scenes[0].trim_out, None);
        assert!(e.scenes[0].cuts.is_empty());
        let ai: Cut = serde_json::from_str(r#"{ "from": 1, "to": 2, "enabled": true, "source": { "kind": "ai", "reason": "2.1 s pause" } }"#).unwrap();
        assert_eq!(ai.source, Source::Ai { reason: "2.1 s pause".into() });
    }
}
