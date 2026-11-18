use lianli_shared::fan::{FanCurve, FanSpeed};
use lianli_shared::installation::{
    CheckState, FindingSeverity, InstallationFinding, InstallationGuide,
};
use lianli_shared::ipc::CurveTemperature;
use lianli_shared::sensors::SensorReading;
use lianli_shared::sensors::SensorSource;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub(super) const TEMPERATURE_GRACE: Duration = Duration::from_secs(5);
pub(super) const FIRST_SAMPLE_WAIT: Duration = Duration::from_secs(3);

pub(super) fn usable_reading(reading: &SensorReading, now: Instant) -> bool {
    reading.value.is_finite()
        && reading.value > 0.0
        && reading.value <= 100.0
        && now.saturating_duration_since(reading.observed_at) < TEMPERATURE_GRACE
}

pub(super) fn curve_sources<'a>(
    speeds: impl IntoIterator<Item = &'a FanSpeed>,
    curves: impl Fn(&str) -> Option<&'a FanCurve>,
) -> HashSet<SensorSource> {
    speeds
        .into_iter()
        .filter_map(|speed| match speed {
            FanSpeed::Curve(name) => curves(name).map(FanCurve::effective_source),
            _ => None,
        })
        .collect()
}

/// Covers only curves evaluated this tick, so an unused curve differs from a lost one.
pub(super) fn current_curve_temperatures(
    evaluated: &HashSet<String>,
    curves: &HashMap<String, FanCurve>,
    temperatures: &HashMap<SensorSource, TemperatureState>,
) -> HashMap<String, CurveTemperature> {
    evaluated
        .iter()
        .filter_map(|name| {
            let source = curves.get(name)?.effective_source();
            let state = temperatures.get(&source)?;
            Some((
                name.clone(),
                CurveTemperature {
                    celsius: state.current,
                    full_speed_fallback: state.fallback,
                    source: Some(source),
                },
            ))
        })
        .collect()
}

pub(super) fn mark_evaluated(evaluated: &mut HashSet<String>, name: &str) {
    if !evaluated.contains(name) {
        evaluated.insert(name.to_owned());
    }
}

// Sampled sources have no value until their background poll first completes. Their
// channels are left as they are until a sample arrives or the wait ends, so that gap
// is not read as a lost temperature.
pub(super) struct FirstSamples {
    pending: HashSet<SensorSource>,
    started: Instant,
    deadline: Instant,
}

impl FirstSamples {
    pub fn new(sources: HashSet<SensorSource>, now: Instant) -> Self {
        Self {
            pending: sources,
            started: now,
            deadline: now + FIRST_SAMPLE_WAIT,
        }
    }

    /// Returns whether any source stopped pending.
    pub fn refresh(
        &mut self,
        now: Instant,
        mut read: impl FnMut(&SensorSource) -> Option<SensorReading>,
    ) -> bool {
        let before = self.pending.len();
        if now >= self.deadline {
            self.pending.clear();
        } else {
            let started = self.started;
            // The monotonic clock stops during suspend, so a pre-suspend sample can look recent.
            self.pending.retain(|source| {
                !read(source).is_some_and(|reading| {
                    reading.observed_at >= started && usable_reading(&reading, now)
                })
            });
        }
        self.pending.len() != before
    }

    pub fn is_pending(&self, source: &SensorSource) -> bool {
        self.pending.contains(source)
    }

    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }
}

pub(super) struct TemperatureState {
    last_valid: Option<(Instant, f32)>,
    pub checked_at: Option<Instant>,
    pub current: Option<f32>,
    pub fallback: bool,
    pub recovered: bool,
}

static TRACKED: AtomicUsize = AtomicUsize::new(0);
static FALLBACK: AtomicUsize = AtomicUsize::new(0);

impl Default for TemperatureState {
    fn default() -> Self {
        TRACKED.fetch_add(1, Ordering::Relaxed);
        Self {
            last_valid: None,
            checked_at: None,
            current: None,
            fallback: false,
            recovered: false,
        }
    }
}

impl Drop for TemperatureState {
    fn drop(&mut self) {
        if self.fallback {
            FALLBACK.fetch_sub(1, Ordering::Relaxed);
        }
        TRACKED.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) fn finding() -> InstallationFinding {
    finding_for(
        TRACKED.load(Ordering::Relaxed),
        FALLBACK.load(Ordering::Relaxed),
    )
}

pub(super) fn missing_curve(
    states: &mut HashMap<String, TemperatureState>,
    name: &str,
    now: Instant,
) -> f32 {
    let state = states.entry(name.into()).or_default();
    if !state.fallback {
        tracing::warn!(
            curve = name,
            "Cooling curve is missing. Affected channels request 100% speed"
        );
    }
    state.update_reading(None, now, 1.0);
    100.0
}

fn finding_for(tracked: usize, fallback: usize) -> InstallationFinding {
    let (state, severity, title, evidence, remediation) = if fallback > 0 {
        (CheckState::Failed, FindingSeverity::Warning, "Cooling fallback active",
            format!("Unavailable temperature sources: {fallback}. Affected channels request 100% speed."),
            "Check temperature sources in Fans and AIO settings, then Recheck.")
    } else if tracked == 0 {
        (
            CheckState::NotApplicable,
            FindingSeverity::Info,
            "Cooling temperatures",
            "No active software temperature curves.".into(),
            "",
        )
    } else {
        (
            CheckState::Passed,
            FindingSeverity::Info,
            "Cooling temperatures",
            "No cooling fallback is active.".into(),
            "",
        )
    };
    InstallationFinding {
        code: "cooling.temperatures".into(),
        state,
        severity,
        feature: "Fan and pump control".into(),
        context: "Selected daemon".into(),
        title: title.into(),
        evidence,
        remediation: remediation.into(),
        guide: InstallationGuide::Troubleshooting,
        message_key: Some(
            if fallback > 0 {
                "daemon.coolingFallback"
            } else {
                "daemon.coolingTemperatures"
            }
            .into(),
        ),
        message_params: BTreeMap::new(),
    }
}

impl TemperatureState {
    pub fn needs_poll(&self, now: Instant) -> bool {
        if self.fallback {
            self.checked_at
                .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_secs(1))
        } else {
            self.last_valid
                .is_none_or(|(at, _)| now.saturating_duration_since(at) >= TEMPERATURE_GRACE)
        }
    }

    #[cfg(test)]
    pub fn update(&mut self, reading: Option<f32>, now: Instant) {
        self.update_smoothed(reading, now, 0.3);
    }

    #[cfg(test)]
    pub fn update_smoothed(&mut self, reading: Option<f32>, now: Instant, alpha: f32) {
        self.update_reading(
            reading.map(|value| SensorReading {
                value,
                observed_at: now,
            }),
            now,
            alpha,
        );
    }

    pub fn update_reading(&mut self, reading: Option<SensorReading>, now: Instant, alpha: f32) {
        let reading = reading.filter(|reading| usable_reading(reading, now));
        self.recovered = self.fallback && reading.is_some();
        if let Some(reading) = reading.filter(|reading| {
            self.last_valid
                .is_none_or(|(at, _)| reading.observed_at > at)
        }) {
            let temp = reading.value;
            let smoothed = self
                .last_valid
                .filter(|(at, _)| now.saturating_duration_since(*at) < TEMPERATURE_GRACE)
                .map_or(temp, |(_, previous)| {
                    alpha * temp + (1.0 - alpha) * previous
                });
            self.last_valid = Some((reading.observed_at, smoothed));
        }
        self.current = self
            .last_valid
            .filter(|(at, _)| now.saturating_duration_since(*at) < TEMPERATURE_GRACE)
            .map(|(_, value)| value);
        let fallback = self.current.is_none();
        if fallback != self.fallback {
            if fallback {
                FALLBACK.fetch_add(1, Ordering::Relaxed);
            } else {
                FALLBACK.fetch_sub(1, Ordering::Relaxed);
            }
        }
        self.fallback = fallback;
        self.checked_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(cmd: &str) -> SensorSource {
        SensorSource::Command { cmd: cmd.into() }
    }

    #[test]
    fn only_curve_slots_with_known_curves_contribute_sources() {
        let curve = FanCurve {
            name: "gpu".into(),
            temp_source: None,
            temp_command: "gpu-temp".into(),
            curve: Vec::new(),
        };
        let speeds = [
            FanSpeed::Curve("gpu".into()),
            FanSpeed::Curve("missing".into()),
            FanSpeed::Constant(128),
            FanSpeed::Curve("gpu".into()),
        ];
        let sources = curve_sources(&speeds, |name| (name == "gpu").then_some(&curve));
        assert_eq!(sources, HashSet::from([command("gpu-temp")]));
    }

    #[test]
    fn each_source_waits_for_its_own_usable_sample_until_the_deadline() {
        let start = Instant::now();
        let (fast, slow) = (command("fast"), command("slow"));
        let mut samples = FirstSamples::new(HashSet::from([fast.clone(), slow.clone()]), start);
        let reading = |value, observed_at| SensorReading { value, observed_at };

        assert!(!samples.refresh(start, |source| {
            (source == &fast).then_some(reading(0.0, start))
        }));
        assert!(samples.is_pending(&fast));

        let now = start + Duration::from_secs(1);
        assert!(samples.refresh(now, |source| {
            (source == &fast).then_some(reading(45.0, now))
        }));
        assert!(!samples.is_pending(&fast));
        assert!(samples.is_pending(&slow));

        let stale = start - TEMPERATURE_GRACE;
        assert!(!samples.refresh(now, |_| Some(reading(45.0, stale))));
        assert!(samples.is_pending(&slow));
        let before_resume = start - Duration::from_millis(1);
        assert!(!samples.refresh(now, |_| Some(reading(45.0, before_resume))));
        assert!(samples.is_pending(&slow));

        assert!(samples.refresh(start + FIRST_SAMPLE_WAIT, |_| None));
        assert!(!samples.waiting());
    }

    #[test]
    fn cooling_findings_distinguish_fallback_recovery_and_unmanaged_channels() {
        assert_eq!(finding_for(0, 0).state, CheckState::NotApplicable);
        let failed = finding_for(3, 1);
        assert_eq!(failed.state, CheckState::Failed);
        assert_eq!(failed.severity, FindingSeverity::Warning);
        assert!(failed.evidence.contains("100%"));
        assert_eq!(finding_for(3, 0).state, CheckState::Passed);
    }

    #[test]
    fn cached_samples_cannot_extend_grace_or_repeat_smoothing() {
        let now = Instant::now();
        let sample = SensorReading {
            value: 40.0,
            observed_at: now,
        };
        let mut state = TemperatureState::default();
        for elapsed in [0, 1000, 2000, 4999] {
            state.update_reading(Some(sample), now + Duration::from_millis(elapsed), 0.3);
            assert_eq!(state.current, Some(40.0));
        }
        state.update_reading(Some(sample), now + TEMPERATURE_GRACE, 0.3);
        assert!(state.fallback);
        assert_eq!(state.current, None);
        let fresh = SensorReading {
            value: 80.0,
            observed_at: now + Duration::from_secs(6),
        };
        state.update_reading(Some(fresh), fresh.observed_at, 0.3);
        assert!(state.recovered);
        assert_eq!(state.current, Some(80.0));
        state.update_reading(Some(sample), fresh.observed_at, 0.3);
        assert_eq!(state.current, Some(80.0));
        state.update_reading(Some(fresh), fresh.observed_at + TEMPERATURE_GRACE, 0.3);
        assert!(state.fallback);
    }
}
