//! The learning loop's numbers (spec-architecture-page): score each member's resolved opinions,
//! propose new council weights, and measure what each guard stopped. All of it is computed here,
//! in code; the language model only explains and ranks what this produces, and nothing changes
//! until the trader applies it.
//!
//! - An opinion is judged by the move [`HORIZON`] later, in ATR: at least [`MOVE_ATR`] in its
//!   direction is a hit, as much against it a miss, anything smaller flat (not scored).
//! - Brier score on P(up) = 0.5 + 0.5 · signed confidence; skill = 1 − Brier / 0.25 (0 is no
//!   better than a coin, 1 perfect). Counterfactual opinions (the decision did not trade) count
//!   half.
//! - Weights: a shrunk multiplicative-weights (Hedge) update, mixed toward equal weights while
//!   the sample is small, clamped to [0.5/N, 2/N], each moving at most 0.05 per review, fitted
//!   on the older 70% and offered only when it does no worse on the newest 30%.
//!
//! The bootstrap interval uses a fixed-seed generator, so the same data give the same numbers.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::AgentId;
use crate::council::Weights;
use crate::opinion::{MEMBERS, Stance};

/// A move this large, in ATR, counts as a direction.
pub const MOVE_ATR: f64 = 0.25;
/// How far ahead opinions are judged, in milliseconds (4 hours).
pub const HORIZON_MS: i64 = 4 * 3600 * 1000;
/// Weight of a counterfactual opinion in the scores.
pub const COUNTERFACTUAL_WEIGHT: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Hit,
    Miss,
    Flat,
}

impl Outcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Outcome::Hit => "hit",
            Outcome::Miss => "miss",
            Outcome::Flat => "flat",
        }
    }
}

/// Judge a stance by the move that followed, in ATR (positive = up).
pub fn judge(stance: Stance, move_atr: f64) -> Outcome {
    let dir = if move_atr >= MOVE_ATR {
        1.0
    } else if move_atr <= -MOVE_ATR {
        -1.0
    } else {
        0.0
    };
    match (stance, dir) {
        (Stance::Neutral, 0.0) => Outcome::Hit,
        (Stance::Neutral, _) => Outcome::Miss,
        (_, 0.0) => Outcome::Flat,
        (s, d) if s.sign() == d => Outcome::Hit,
        _ => Outcome::Miss,
    }
}

/// One member's opinion with what the price did afterwards.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolved {
    pub decision_id: String,
    pub agent: AgentId,
    pub stance: Stance,
    pub confidence: u8,
    pub move_atr: f64,
    /// The decision did not trade.
    pub counterfactual: bool,
    pub at_ms: i64,
}

impl Resolved {
    fn signed(&self) -> f64 {
        self.stance.sign() * f64::from(self.confidence) / 100.0
    }

    /// `Some((weight, brier))` when the opinion took a side and the price moved enough.
    fn brier(&self) -> Option<(f64, f64)> {
        if self.stance == Stance::Neutral || self.confidence == 0 {
            return None;
        }
        let y = if self.move_atr >= MOVE_ATR {
            1.0
        } else if self.move_atr <= -MOVE_ATR {
            0.0
        } else {
            return None;
        };
        let p = 0.5 + 0.5 * self.signed();
        let w = if self.counterfactual {
            COUNTERFACTUAL_WEIGHT
        } else {
            1.0
        };
        Some((w, (p - y) * (p - y)))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberStats {
    pub agent: AgentId,
    /// Scored opinions: took a side, and the price moved enough to judge.
    pub n: usize,
    pub n_taken: usize,
    pub n_counterfactual: usize,
    /// Opinions that were neutral (not scored).
    pub neutral: usize,
    /// Took a side but the price did not move enough (not scored).
    pub flat: usize,
    pub hit_rate: Option<f64>,
    pub hit_rate_taken: Option<f64>,
    pub hit_rate_counterfactual: Option<f64>,
    pub brier: Option<f64>,
    pub skill: Option<f64>,
    /// 80% bootstrap interval of the skill.
    pub skill_low: Option<f64>,
    pub skill_high: Option<f64>,
}

fn weighted_skill(items: &[(f64, f64)]) -> Option<f64> {
    let w: f64 = items.iter().map(|(w, _)| w).sum();
    (w > 0.0).then(|| 1.0 - items.iter().map(|(w, b)| w * b).sum::<f64>() / w / 0.25)
}

/// A small fixed-seed generator for the bootstrap (no randomness from outside).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % n as u64) as usize
    }
}

fn bootstrap(items: &[(f64, f64)]) -> Option<(f64, f64)> {
    if items.len() < 5 {
        return None;
    }
    let mut rng = Lcg(0x9E37_79B9_7F4A_7C15 ^ items.len() as u64);
    let mut skills: Vec<f64> = (0..500)
        .filter_map(|_| {
            let sample: Vec<(f64, f64)> = (0..items.len())
                .map(|_| items[rng.next(items.len())])
                .collect();
            weighted_skill(&sample)
        })
        .collect();
    skills.sort_by(f64::total_cmp);
    let at = |p: f64| skills[((skills.len() - 1) as f64 * p).round() as usize];
    Some((round3(at(0.1)), round3(at(0.9))))
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn rate(hits: usize, n: usize) -> Option<f64> {
    (n > 0).then(|| round3(hits as f64 / n as f64))
}

/// Each member's record over the resolved opinions.
pub fn member_stats(resolved: &[Resolved]) -> Vec<MemberStats> {
    MEMBERS
        .iter()
        .map(|agent| {
            let mine: Vec<&Resolved> = resolved.iter().filter(|r| r.agent == *agent).collect();
            let scored: Vec<&Resolved> = mine
                .iter()
                .copied()
                .filter(|r| r.brier().is_some())
                .collect();
            let items: Vec<(f64, f64)> = scored.iter().filter_map(|r| r.brier()).collect();
            let hits = |rs: &[&Resolved]| {
                rs.iter()
                    .filter(|r| judge(r.stance, r.move_atr) == Outcome::Hit)
                    .count()
            };
            let taken: Vec<&Resolved> = scored
                .iter()
                .copied()
                .filter(|r| !r.counterfactual)
                .collect();
            let cf: Vec<&Resolved> = scored
                .iter()
                .copied()
                .filter(|r| r.counterfactual)
                .collect();
            let ci = bootstrap(&items);
            MemberStats {
                agent: *agent,
                n: scored.len(),
                n_taken: taken.len(),
                n_counterfactual: cf.len(),
                neutral: mine
                    .iter()
                    .filter(|r| r.stance == Stance::Neutral || r.confidence == 0)
                    .count(),
                flat: mine
                    .iter()
                    .filter(|r| {
                        r.stance != Stance::Neutral && r.confidence > 0 && r.brier().is_none()
                    })
                    .count(),
                hit_rate: rate(hits(&scored), scored.len()),
                hit_rate_taken: rate(hits(&taken), taken.len()),
                hit_rate_counterfactual: rate(hits(&cf), cf.len()),
                brier: (!items.is_empty()).then(|| {
                    let w: f64 = items.iter().map(|(w, _)| w).sum();
                    round3(items.iter().map(|(w, b)| w * b).sum::<f64>() / w)
                }),
                skill: weighted_skill(&items).map(round3),
                skill_low: ci.map(|c| c.0),
                skill_high: ci.map(|c| c.1),
            }
        })
        .collect()
}

/// The update's knobs. `Default` is the approved design.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnConfig {
    pub eta: f64,
    /// Shrinkage: a member's skill counts n / (n + k).
    pub k: f64,
    /// Full trust in the learned weights from this many scored opinions on average.
    pub full_trust_n: f64,
    /// Bounds as multiples of 1/N.
    pub floor: f64,
    pub cap: f64,
    /// Largest move of one weight per review.
    pub max_step: f64,
    /// Scored opinions a member needs before its weight may change.
    pub min_n: usize,
    /// Share of the newest data held out to check a proposal.
    pub holdout: f64,
    /// Decisions the holdout needs at least.
    pub min_holdout: usize,
}

impl Default for LearnConfig {
    fn default() -> Self {
        Self {
            eta: 0.5,
            k: 50.0,
            full_trust_n: 100.0,
            floor: 0.5,
            cap: 2.0,
            max_step: 0.05,
            min_n: 30,
            holdout: 0.3,
            min_holdout: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightChange {
    pub agent: AgentId,
    pub old: f64,
    pub new: f64,
    pub skill: Option<f64>,
    pub n: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Holdout {
    pub decisions: usize,
    pub brier_old: f64,
    pub brier_new: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeightProposal {
    pub current: Weights,
    pub proposed: Weights,
    pub changes: Vec<WeightChange>,
    pub holdout: Option<Holdout>,
    /// Whether it may be offered for one-click apply.
    pub eligible: bool,
    /// Why not, or what limited it, in plain words.
    pub notes: Vec<String>,
}

/// The council's Brier score over decisions (one P(up) per decision from the weighted votes).
fn ensemble_brier(rows: &[&Resolved], w: &Weights) -> Option<(usize, f64)> {
    let mut by: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for r in rows {
        let e = by
            .entry(r.decision_id.as_str())
            .or_insert((0.0, r.move_atr));
        e.0 += w.get(r.agent) * r.signed();
    }
    let scored: Vec<f64> = by
        .values()
        .filter_map(|(score, mv)| {
            let y = if *mv >= MOVE_ATR {
                1.0
            } else if *mv <= -MOVE_ATR {
                0.0
            } else {
                return None;
            };
            let p = (0.5 + 0.5 * score).clamp(0.0, 1.0);
            Some((p - y) * (p - y))
        })
        .collect();
    (!scored.is_empty()).then(|| {
        (
            scored.len(),
            scored.iter().sum::<f64>() / scored.len() as f64,
        )
    })
}

/// Propose new council weights from the resolved opinions.
pub fn propose_weights(
    current: &Weights,
    resolved: &[Resolved],
    cfg: &LearnConfig,
) -> WeightProposal {
    let current = current.normalised();
    let n_members = MEMBERS.len() as f64;
    let mut sorted: Vec<&Resolved> = resolved.iter().collect();
    sorted.sort_by_key(|r| r.at_ms);
    // Split by decision time, so a decision's opinions stay on one side.
    let mut seen = std::collections::HashSet::new();
    let mut ids: Vec<&str> = Vec::new();
    for r in &sorted {
        if seen.insert(r.decision_id.as_str()) {
            ids.push(&r.decision_id);
        }
    }
    let cut = ((ids.len() as f64) * (1.0 - cfg.holdout)).floor() as usize;
    let train_ids: std::collections::HashSet<&str> = ids[..cut].iter().copied().collect();
    let (train, test): (Vec<&Resolved>, Vec<&Resolved>) = sorted
        .iter()
        .copied()
        .partition(|r| train_ids.contains(&r.decision_id.as_str()));
    let train_owned: Vec<Resolved> = train.iter().map(|r| (*r).clone()).collect();
    let train_stats = member_stats(&train_owned);
    let all_stats = member_stats(resolved);

    let mut notes = Vec::new();
    let raw: BTreeMap<AgentId, f64> = train_stats
        .iter()
        .map(|s| {
            let n = s.n as f64;
            let skill = s.skill.unwrap_or(0.0);
            (s.agent, (cfg.eta * n / (n + cfg.k) * skill).exp())
        })
        .collect();
    let raw_sum: f64 = raw.values().sum();
    let mean_n = train_stats.iter().map(|s| s.n as f64).sum::<f64>() / n_members;
    let lambda = (mean_n / cfg.full_trust_n).min(1.0);
    let (lo, hi) = (cfg.floor / n_members, cfg.cap / n_members);

    let mut proposed: BTreeMap<AgentId, f64> = BTreeMap::new();
    for s in &all_stats {
        let old = current.get(s.agent);
        if s.n < cfg.min_n {
            notes.push(format!(
                "{} has {} of {} scored opinions, so its weight stays.",
                s.agent, s.n, cfg.min_n
            ));
            proposed.insert(s.agent, old);
            continue;
        }
        let target = lambda * raw[&s.agent] / raw_sum + (1.0 - lambda) / n_members;
        let step = (target - old).clamp(-cfg.max_step, cfg.max_step);
        proposed.insert(s.agent, (old + step).clamp(lo, hi));
    }
    // Renormalise, then keep the bounds (two passes are enough for six members).
    let mut w = Weights(proposed).normalised();
    for _ in 0..2 {
        w = Weights(w.0.iter().map(|(a, v)| (*a, v.clamp(lo, hi))).collect()).normalised();
    }
    let w = Weights(w.0.iter().map(|(a, v)| (*a, round3(*v))).collect());

    let changes: Vec<WeightChange> = all_stats
        .iter()
        .filter_map(|s| {
            let (old, new) = (current.get(s.agent), w.get(s.agent));
            ((new - old).abs() >= 0.005).then(|| WeightChange {
                agent: s.agent,
                old: round3(old),
                new,
                skill: s.skill,
                n: s.n,
            })
        })
        .collect();

    let holdout = match (ensemble_brier(&test, &current), ensemble_brier(&test, &w)) {
        (Some((n, old)), Some((_, new))) => Some(Holdout {
            decisions: n,
            brier_old: round3(old),
            brier_new: round3(new),
        }),
        _ => None,
    };
    let holdout_ok = match &holdout {
        Some(h) if h.decisions >= cfg.min_holdout => {
            if h.brier_new > h.brier_old + 1e-9 {
                notes.push(format!(
                    "On the newest decisions the new weights did worse ({} vs {}).",
                    h.brier_new, h.brier_old
                ));
                false
            } else {
                true
            }
        }
        _ => {
            notes.push(format!(
                "Too few recent decisions to check against (need {}).",
                cfg.min_holdout
            ));
            false
        }
    };
    if changes.is_empty() {
        notes.push("No weight would move by half a point or more.".into());
    }
    WeightProposal {
        current,
        eligible: holdout_ok && !changes.is_empty(),
        proposed: w,
        changes,
        holdout,
        notes,
    }
}

/// What the price did after a decision, from H1 bars on the bars' own clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MoveAfter {
    /// The move in ATR from `price` to the close of the last bar completed by the horizon.
    Move(f64),
    /// The horizon has not passed in the bars yet.
    NotYet,
    /// The bars do not reach back to the decision (or no ATR), so it cannot be judged.
    Unjudgeable,
}

/// `t0` is the decision time in seconds on the bars' clock; bars are H1, oldest first.
pub fn move_after(bars: &[jarvis_market::Bar], t0: i64, price: f64, atr: Option<f64>) -> MoveAfter {
    const H1: i64 = 3600;
    let horizon = t0 + HORIZON_MS / 1000;
    let Some(last) = bars.last() else {
        return MoveAfter::NotYet;
    };
    if last.time + H1 < horizon {
        return MoveAfter::NotYet;
    }
    let before: Vec<jarvis_market::Bar> = bars.iter().filter(|b| b.time < t0).cloned().collect();
    if before.is_empty() {
        return MoveAfter::Unjudgeable;
    }
    let atr = atr.filter(|a| *a > 0.0).or_else(|| {
        jarvis_market::indicators::atr(&before, 14)
            .into_iter()
            .flatten()
            .last()
            .filter(|a| *a > 0.0)
    });
    let Some(atr) = atr else {
        return MoveAfter::Unjudgeable;
    };
    match bars
        .iter()
        .rev()
        .find(|b| b.time >= t0 && b.time + H1 <= horizon)
    {
        Some(b) => MoveAfter::Move(round3((b.close - price) / atr)),
        None => MoveAfter::Unjudgeable,
    }
}

/// A decision's ending, for the guard statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionEnd {
    pub stop_code: Option<String>,
    pub traded: bool,
    /// The council's score (its sign is the side it leaned to).
    pub score: Option<f64>,
    /// The move afterwards in ATR, when resolved.
    pub move_atr: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GuardStat {
    pub code: String,
    pub stops: usize,
    pub resolved: usize,
    /// Mean move in the council's direction after the stop, in ATR: positive means the stop
    /// missed a move, negative that it avoided one. Counterfactual: no fills, no costs.
    pub mean_move_atr: Option<f64>,
}

/// How often each stop code ended a decision and what the price did next.
pub fn guard_stats(ends: &[DecisionEnd]) -> Vec<GuardStat> {
    let mut by: BTreeMap<String, (usize, Vec<f64>)> = BTreeMap::new();
    for e in ends.iter().filter(|e| !e.traded) {
        let Some(code) = &e.stop_code else { continue };
        let entry = by.entry(code.clone()).or_default();
        entry.0 += 1;
        if let (Some(score), Some(mv)) = (e.score, e.move_atr)
            && score != 0.0
        {
            entry.1.push(score.signum() * mv);
        }
    }
    let mut out: Vec<GuardStat> = by
        .into_iter()
        .map(|(code, (stops, moves))| GuardStat {
            code,
            stops,
            resolved: moves.len(),
            mean_move_atr: (!moves.is_empty())
                .then(|| round3(moves.iter().sum::<f64>() / moves.len() as f64)),
        })
        .collect();
    out.sort_by(|a, b| b.stops.cmp(&a.stops).then(a.code.cmp(&b.code)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(i: usize, agent: AgentId, stance: Stance, conf: u8, mv: f64) -> Resolved {
        Resolved {
            decision_id: format!("d{i}"),
            agent,
            stance,
            confidence: conf,
            move_atr: mv,
            counterfactual: i.is_multiple_of(2),
            at_ms: i as i64,
        }
    }

    fn bar(time: i64, close: f64) -> jarvis_market::Bar {
        jarvis_market::Bar {
            time,
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            tick_volume: 1,
            spread: 0,
        }
    }

    #[test]
    fn the_move_is_read_at_the_horizon_in_atr() {
        let bars: Vec<_> = (0..30).map(|i| bar(i * 3600, 100.0 + i as f64)).collect();
        // Decision at bar 20 (price 120); horizon 4 h later: the last bar completed is 23.
        match move_after(&bars, 20 * 3600, 120.0, Some(2.0)) {
            MoveAfter::Move(m) => assert!((m - 1.5).abs() < 1e-9, "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            move_after(&bars, 28 * 3600, 128.0, Some(2.0)),
            MoveAfter::NotYet
        );
        assert_eq!(
            move_after(&bars, -100, 100.0, Some(2.0)),
            MoveAfter::Unjudgeable
        );
        // Without an ATR it is computed from the bars before the decision (range 2 here).
        match move_after(&bars, 20 * 3600, 120.0, None) {
            MoveAfter::Move(m) => assert!(m > 0.0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn judging_needs_a_quarter_atr() {
        assert_eq!(judge(Stance::Bull, 0.3), Outcome::Hit);
        assert_eq!(judge(Stance::Bull, -0.3), Outcome::Miss);
        assert_eq!(judge(Stance::Bear, 0.1), Outcome::Flat);
        assert_eq!(judge(Stance::Neutral, 0.1), Outcome::Hit);
        assert_eq!(judge(Stance::Neutral, 0.5), Outcome::Miss);
    }

    /// Oracle is right 80% of the time, Chartist 30%, the others always neutral.
    fn history(n: usize) -> Vec<Resolved> {
        let mut out = Vec::new();
        for i in 0..n {
            let mv = if i % 2 == 0 { 0.6 } else { -0.6 };
            let up = mv > 0.0;
            let oracle_right = i % 5 != 0;
            let chartist_right = i % 10 < 3;
            let side = |right: bool| {
                if right == up {
                    Stance::Bull
                } else {
                    Stance::Bear
                }
            };
            out.push(r(i, AgentId::Oracle, side(oracle_right), 60, mv));
            out.push(r(i, AgentId::Chartist, side(chartist_right), 60, mv));
            for a in [
                AgentId::Atlas,
                AgentId::Capitol,
                AgentId::Scout,
                AgentId::Athena,
            ] {
                out.push(r(i, a, Stance::Neutral, 0, mv));
            }
        }
        out
    }

    #[test]
    fn stats_score_skill_and_split_taken_from_counterfactual() {
        let stats = member_stats(&history(100));
        let oracle = stats.iter().find(|s| s.agent == AgentId::Oracle).unwrap();
        assert_eq!(oracle.n, 100);
        assert_eq!(oracle.n_taken + oracle.n_counterfactual, 100);
        assert_eq!(oracle.hit_rate, Some(0.8));
        assert!(oracle.skill.unwrap() > 0.2, "{oracle:?}");
        assert!(oracle.skill_low.unwrap() <= oracle.skill.unwrap());
        assert!(oracle.skill_high.unwrap() >= oracle.skill.unwrap());
        let chartist = stats.iter().find(|s| s.agent == AgentId::Chartist).unwrap();
        assert!(chartist.skill.unwrap() < 0.0);
        let atlas = stats.iter().find(|s| s.agent == AgentId::Atlas).unwrap();
        assert_eq!((atlas.n, atlas.neutral), (0, 100));
        assert_eq!(atlas.skill, None);
        // Deterministic: the same data give the same interval.
        assert_eq!(member_stats(&history(100)), stats);
    }

    #[test]
    fn weights_move_toward_skill_within_the_step_and_bounds() {
        let p = propose_weights(&Weights::default(), &history(200), &LearnConfig::default());
        let old = 1.0 / 6.0;
        let (o, c) = (
            p.proposed.get(AgentId::Oracle),
            p.proposed.get(AgentId::Chartist),
        );
        assert!(o > old && c < old, "{p:?}");
        for w in p.proposed.0.values() {
            assert!((0.5 / 6.0 - 1e-3..=2.0 / 6.0 + 1e-3).contains(w), "{p:?}");
        }
        assert!((p.proposed.0.values().sum::<f64>() - 1.0).abs() < 0.01);
        assert!(
            p.changes
                .iter()
                .all(|c| (c.new - c.old).abs() <= 0.05 + 0.02)
        );
        assert!(p.eligible, "{:?}", p.notes);
        let h = p.holdout.unwrap();
        assert!(h.brier_new <= h.brier_old);
    }

    #[test]
    fn small_samples_change_nothing() {
        let p = propose_weights(&Weights::default(), &history(20), &LearnConfig::default());
        assert!(p.changes.is_empty(), "{p:?}");
        assert!(!p.eligible);
        assert!(p.notes.iter().any(|n| n.contains("of 30")));
    }

    #[test]
    fn guard_stats_show_what_a_stop_missed_or_avoided() {
        let ends = vec![
            DecisionEnd {
                stop_code: Some("council_below_threshold".into()),
                traded: false,
                score: Some(0.2),
                move_atr: Some(-0.5),
            },
            DecisionEnd {
                stop_code: Some("council_below_threshold".into()),
                traded: false,
                score: Some(-0.1),
                move_atr: Some(-0.3),
            },
            DecisionEnd {
                stop_code: Some("sentinel_veto:max_open_positions".into()),
                traded: false,
                score: Some(0.4),
                move_atr: None,
            },
            DecisionEnd {
                stop_code: None,
                traded: true,
                score: Some(0.5),
                move_atr: Some(1.0),
            },
        ];
        let g = guard_stats(&ends);
        assert_eq!(g[0].code, "council_below_threshold");
        assert_eq!((g[0].stops, g[0].resolved), (2, 2));
        assert_eq!(g[0].mean_move_atr, Some(-0.1)); // (-0.5 + 0.3) / 2
        assert_eq!(g[1].mean_move_atr, None);
        assert_eq!(g.len(), 2);
    }
}
