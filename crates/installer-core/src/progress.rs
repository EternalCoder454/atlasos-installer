//! Install progress, weighted by stage, and the time left. bootc install has
//! no progress stream, and without a terminal it prints only a few milestone
//! lines (captured in spike S3 and checked in the VM). So the copy stage
//! moves on those, and in between on the bytes and layers the helper sees
//! copied (or, when it can't count them, on elapsed time).

use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Prepare,
    Partition,
    Format,
    Copy,
    Deploy,
    Bootloader,
    Settings,
    Finish,
}

impl Stage {
    /// The `step` of the Progress signal.
    pub fn id(self) -> &'static str {
        match self {
            Stage::Prepare => "prepare",
            Stage::Partition => "partition",
            Stage::Format => "format",
            Stage::Copy => "copy",
            Stage::Deploy => "deploy",
            Stage::Bootloader => "bootloader",
            Stage::Settings => "settings",
            Stage::Finish => "finish",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Stage::Prepare => "Getting ready",
            Stage::Partition => "Creating partitions",
            Stage::Format => "Formatting",
            Stage::Copy => "Copying AtlasOS",
            Stage::Deploy => "Setting up AtlasOS",
            Stage::Bootloader => "Installing the bootloader",
            Stage::Settings => "Applying your settings",
            Stage::Finish => "Finishing up",
        }
    }

    /// The share of the whole bar, as (start, end).
    pub fn range(self) -> (f64, f64) {
        match self {
            Stage::Prepare => (0.00, 0.02),
            Stage::Partition => (0.02, 0.04),
            Stage::Format => (0.04, 0.07),
            Stage::Copy => (0.07, 0.77),
            Stage::Deploy => (0.77, 0.85),
            Stage::Bootloader => (0.85, 0.93),
            Stage::Settings => (0.93, 0.97),
            Stage::Finish => (0.97, 1.00),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Progress {
    pub stage: Stage,
    /// Of the whole install, 0 to 1.
    pub fraction: f64,
}

impl Progress {
    /// `within` (0 to 1) of `stage`.
    pub fn at(stage: Stage, within: f64) -> Progress {
        let (a, b) = stage.range();
        Progress {
            stage,
            fraction: a + (b - a) * within.clamp(0.0, 1.0),
        }
    }

    pub fn text(&self) -> &'static str {
        self.stage.text()
    }
}

/// Follows `bootc install to-filesystem`'s output, and, while the layers
/// import, what has been copied (see [`BootcProgress::counted`]). Never goes
/// backwards.
#[derive(Debug, Clone)]
pub struct BootcProgress {
    last: Progress,
    /// When the layer import started, and how long it is expected to take:
    /// the bar eases forward with time until a byte count arrives.
    import: Option<(Duration, Duration)>,
    /// Layers already in the new system, and layers to import, from bootc.
    layers: Option<(u64, u64)>,
    /// Counts arrived: they move the bar, not the time.
    measured: bool,
    /// When everything was counted in (or bootc moved on): bootc is merging
    /// and deploying the layers, and says nothing until it is done.
    copied: Option<Duration>,
    /// The import's share, and since when it has stood there.
    still: Option<(f64, Duration)>,
    /// The import is over (bootc is deploying, or further on).
    imported: bool,
    /// When progress last went out.
    sent: Duration,
}

/// Where the layer import runs within the copy stage: from the "layers
/// needed" line to its end. The deploy after it is a stage of its own.
const IMPORT_FROM: f64 = 0.06;
const IMPORT_TO: f64 = 1.0;
/// Without counts, the import eases toward here: the end is for the counts
/// (or bootc's deploy line) to say.
const EASE_TO: f64 = 0.9;

/// Seconds of import per GB (of bootc's "layers needed" figure), for the bar
/// while no byte count is available. A guess: 24 in a VM reading from a CD,
/// 243 in a VM reading from USB through LUKS.
const SECS_PER_GB: f64 = 60.0;

/// The least count that shows the layers are being counted (with a thousandth
/// of the image, if that is more).
const MEANINGFUL_BYTES: u64 = 1 << 20;

/// Counted this far, the bytes are all in.
const COPIED: f64 = 0.995;

/// When the counts stand still this long at the very end, the layers are
/// in, whatever the totals say: a layer bootc lists twice gets one ref, and
/// without a layer count, a size from podman can be off.
const STILL: Duration = Duration::from_secs(30);
/// Without a layer count, "the very end".
const NEARLY: f64 = 0.95;

/// After the last layer, bootc merges and deploys them before its next line
/// ("Deploying container image...done", printed at the end): the bar eases
/// over about this long (2 and 3 minutes in the VMs).
const DEPLOY_SECS: f64 = 120.0;

/// While bootc runs, progress goes out at least this often, even unchanged,
/// so the UI's time estimate keeps up when nothing moves (the deploy).
const HEARTBEAT: Duration = Duration::from_secs(5);

impl Default for BootcProgress {
    fn default() -> Self {
        BootcProgress {
            last: Progress::at(Stage::Copy, 0.0),
            import: None,
            layers: None,
            measured: false,
            copied: None,
            still: None,
            imported: false,
            sent: Duration::ZERO,
        }
    }
}

impl BootcProgress {
    pub fn current(&self) -> &Progress {
        &self.last
    }

    /// The layers are all in, by the counts or by bootc's deploy line.
    pub fn copied(&self) -> bool {
        self.copied.is_some() || self.imported
    }

    fn advance(&mut self, p: Progress, now: Duration) -> Option<Progress> {
        if p.fraction > self.last.fraction + 1e-4 || p.stage > self.last.stage {
            if p.fraction >= self.last.fraction {
                self.last = p;
            } else {
                self.last.stage = p.stage;
            }
            self.sent = now;
            Some(self.last.clone())
        } else {
            None
        }
    }

    /// A line of bootc output at time `now` (since bootc started).
    pub fn line(&mut self, line: &str, now: Duration) -> Option<Progress> {
        let l = line.trim();
        let p = if l.starts_with("Installing image") {
            Progress::at(Stage::Copy, 0.02)
        } else if l.starts_with("Initializing ostree layout") {
            Progress::at(Stage::Copy, 0.04)
        } else if l.starts_with("layers already present") {
            let gb = layers_gb(l).unwrap_or(3.0);
            let expect = Duration::from_secs_f64((gb * SECS_PER_GB).max(10.0));
            self.import = Some((now, expect));
            self.layers = layer_counts(l);
            Progress::at(Stage::Copy, IMPORT_FROM)
        } else if l.starts_with("Deploying container image") {
            self.imported = true;
            Progress::at(Stage::Deploy, if l.contains("done") { 1.0 } else { 0.5 })
        } else if l.starts_with("Bootloader:") || l.starts_with("Installing bootloader") {
            self.imported = true;
            Progress::at(Stage::Bootloader, 0.1)
        } else if l.starts_with("Trimming") {
            Progress::at(Stage::Bootloader, 0.6)
        } else if l.starts_with("Finalizing") {
            Progress::at(Stage::Bootloader, 0.8)
        } else if l.starts_with("Installation complete") {
            Progress::at(Stage::Bootloader, 1.0)
        } else {
            return None;
        };
        self.advance(p, now)
    }

    /// What the helper has counted at `now`: `bytes` copied of the image's
    /// total (if it knows the total), and the layer refs in the new system.
    /// The import moves by the average share of the two, from the first
    /// meaningful count on: bytes alone run fast through large files and slow
    /// through small ones, while layers differ in size. Ignored outside the
    /// import, and until something has been counted (until then, and without
    /// counts, the time-based easing goes on).
    pub fn counted(
        &mut self,
        bytes: Option<(u64, u64)>,
        layers: u64,
        now: Duration,
    ) -> Option<Progress> {
        if self.imported
            || self.copied.is_some()
            || self.import.is_none()
            || self.last.stage != Stage::Copy
        {
            return None;
        }
        let bytes = bytes.filter(|(_, total)| *total > 0);
        let layers = self
            .layers
            .filter(|(_, needed)| *needed > 0)
            .map(|(present, needed)| (layers.saturating_sub(present), needed));
        if !self.measured {
            // a few kB is the proxy's own talk, not layers: until a real
            // amount has gone through, or a layer is in, the count may not be
            // counting the copy at all
            let some_bytes = bytes.is_some_and(|(d, t)| d >= MEANINGFUL_BYTES.max(t / 1000));
            let some_layers = layers.is_some_and(|(d, _)| d > 0);
            if !some_bytes && !some_layers {
                return None;
            }
            self.measured = true;
        }
        let shares: Vec<f64> = [bytes, layers]
            .into_iter()
            .flatten()
            .map(|(d, t)| (d as f64 / t as f64).min(1.0))
            .collect();
        if shares.is_empty() {
            return None;
        }
        let share = shares.iter().sum::<f64>() / shares.len() as f64;
        if self.still.is_none_or(|(s, _)| (share - s).abs() > 1e-9) {
            self.still = Some((share, now));
        }
        let quiet = self
            .still
            .is_some_and(|(_, since)| now.saturating_sub(since) >= STILL);
        // the layer count is exact, so it decides when there is one: the
        // bytes can be all in with the last layer still importing, and a
        // pause with a few layers left is a pause
        let all_bytes = bytes.is_some_and(|(d, t)| d as f64 / t as f64 >= COPIED);
        let all_in = match layers {
            Some((d, n)) => d >= n || (quiet && all_bytes && d + 1 >= n),
            None => all_bytes || (quiet && share >= NEARLY),
        };
        if all_in {
            self.copied = Some(now);
            return self.advance(Progress::at(Stage::Deploy, 0.0), now);
        }
        let within = IMPORT_FROM + (IMPORT_TO - IMPORT_FROM) * share;
        self.advance(Progress::at(Stage::Copy, within), now)
    }

    /// Time passes: without counts the import eases toward the end of the
    /// stage, and once all is counted, the deploy toward its end. Unchanged
    /// progress is sent again every [`HEARTBEAT`].
    pub fn tick(&mut self, now: Duration) -> Option<Progress> {
        // 1 - e^-t: 63 % of the way at the expected time, never arriving
        let ease = |since: Duration, expect: f64| 1.0 - (-since.as_secs_f64() / expect).exp();
        let next = match (self.import, self.copied) {
            _ if self.imported => None,
            (_, Some(at)) => Some(Progress::at(
                Stage::Deploy,
                0.99 * ease(now.saturating_sub(at), DEPLOY_SECS),
            )),
            (Some((start, expect)), None) if !self.measured => Some(Progress::at(
                Stage::Copy,
                IMPORT_FROM
                    + (EASE_TO - IMPORT_FROM)
                        * ease(now.saturating_sub(start), expect.as_secs_f64()),
            )),
            _ => None,
        };
        if let Some(next) = next
            && let Some(p) = self.advance(next, now)
        {
            return Some(p);
        }
        if now.saturating_sub(self.sent) >= HEARTBEAT {
            self.sent = now;
            return Some(self.last.clone());
        }
        None
    }
}

/// How long the install takes after the last layer: bootc merges and deploys
/// the layers, silently, then installs the bootloader, and the settings and
/// the final relabel follow. It grows with the import, which is slow where
/// the disks are: 170 s after an 1080 s import in one VM, 125 s after a
/// 546 s one in another (69 s of merge, 52 of deploy, 4 for the rest).
const AFTER_BASE_SECS: f64 = 80.0;
const AFTER_PER_IMPORT: f64 = 0.085;
/// Without a timed import (the UI started following after it).
const AFTER_SECS: f64 = 125.0;
/// From bootc's "Deploying container image...done" to the end.
const POST_SECS: f64 = 10.0;
/// The copy speed is a moving average over about this long. The copy runs
/// in bursts (from 1 to 24 MB/s, half a minute at a time, in the VM), which
/// a shorter average turns into estimates from 7 minutes to 2 hours.
const RATE_SECS: f64 = 180.0;
/// For its first this long, the speed is the average since the start.
const SEED_SECS: f64 = 30.0;
/// The copy is watched this long before its speed is believed.
const WARMUP_SECS: f64 = 15.0;
/// Past this, the copy has (nearly) stopped, and no estimate is shown.
const STALLED_SECS: f64 = 3.0 * 3600.0;
/// The countdown moves toward a changed estimate over about this long, so it
/// ticks down steadily instead of jumping with each burst of the copy.
const SMOOTH_SECS: f64 = 20.0;

/// What the estimate says.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Estimate {
    Calculating,
    /// Seconds left.
    Secs(f64),
    AlmostDone,
}

/// "03:06 left", from the copy's measured speed: what is left of the import
/// at the average speed of the last few minutes, plus the time after it
/// (see [`after_import`]), counted again from the last layer and from the
/// end of the deploy. It counts down a second a second between estimates,
/// and moves toward a new one over [`SMOOTH_SECS`]. Fed the Progress
/// fraction, and ticked for the clock, with the time since the UI started
/// following.
#[derive(Debug, Clone, Default)]
pub struct TimeLeft {
    /// The last sample in the import: fraction and time.
    last: Option<(f64, Duration)>,
    /// The first sample in the import: fraction and time.
    began: Option<(f64, Duration)>,
    /// When the deploy began (the import ended), in seconds on the caller's
    /// clock (before it began, for a UI that followed late), and when it
    /// ended.
    ended: Option<f64>,
    deployed: Option<Duration>,
    /// Smoothed fraction per second during the import.
    rate: Option<f64>,
    /// How long the whole import takes, as last projected, in seconds.
    import: Option<f64>,
    /// When the install should be done (seconds on the caller's clock), as
    /// shown: following the estimate, smoothed.
    finish: Option<f64>,
    /// When `finish` last moved toward the estimate.
    moved: Duration,
    /// The estimate at the last progress.
    shown: Option<Estimate>,
    /// The clock reached zero before the install was done: "Almost done"
    /// stays until an estimate well past it comes.
    ran_out: bool,
}

/// After the clock has run out, the least time left that brings it back.
const BACK_SECS: f64 = 30.0;

impl TimeLeft {
    /// The text for a new Progress `fraction` at `now`.
    pub fn text(&mut self, fraction: f64, now: Duration) -> String {
        let est = self.estimate(fraction, now);
        let t = now.as_secs_f64();
        if let Estimate::Secs(left) = est {
            let target = t + left;
            let dt = now.saturating_sub(self.moved).as_secs_f64();
            self.finish = Some(match self.finish {
                Some(f) => f + (target - f) * (1.0 - (-dt / SMOOTH_SECS).exp()),
                None => target,
            });
            self.moved = now;
            if self.finish.is_some_and(|f| f - t > BACK_SECS) {
                self.ran_out = false;
            }
        } else {
            self.finish = None;
            self.ran_out = false;
        }
        self.shown = Some(est);
        self.tick(now)
    }

    /// The text at `now`, with no new progress: the clock runs down.
    pub fn tick(&mut self, now: Duration) -> String {
        match (self.shown, self.finish) {
            (Some(Estimate::Secs(_)), Some(f)) => {
                let left = (f - now.as_secs_f64()).round();
                if left < 1.0 || self.ran_out {
                    // the estimate ran out before the install did
                    self.ran_out = true;
                    "Almost done".into()
                } else {
                    format!("{} left", clock(left as u64))
                }
            }
            (Some(Estimate::AlmostDone), _) => "Almost done".into(),
            _ => "Calculating the time left…".into(),
        }
    }

    fn estimate(&mut self, fraction: f64, now: Duration) -> Estimate {
        let from = Progress::at(Stage::Copy, IMPORT_FROM).fraction;
        let to = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        let since = |at: Duration| now.saturating_sub(at).as_secs_f64();
        if fraction >= Stage::Finish.range().0 {
            return Estimate::AlmostDone;
        }
        if fraction >= Stage::Bootloader.range().0 - 1e-9 {
            if self.deployed.is_none() {
                // known to the second: the clock goes straight to it
                self.finish = None;
                self.ran_out = false;
            }
            let deployed = *self.deployed.get_or_insert(now);
            return secs(POST_SECS - since(deployed));
        }
        if fraction >= to - 1e-9 {
            let ended = *self.ended.get_or_insert_with(|| {
                if self.began.is_some() {
                    return now.as_secs_f64();
                }
                // followed from the deploy on: how long it has run, from how
                // far BootcProgress has eased it
                let (a, b) = Stage::Deploy.range();
                let eased = ((fraction - a) / (b - a) / 0.99).clamp(0.0, 0.99);
                now.as_secs_f64() + DEPLOY_SECS * (1.0 - eased).ln()
            });
            return secs(after_import(self.import) - (now.as_secs_f64() - ended));
        }
        if fraction < from - 1e-9 {
            return Estimate::Calculating;
        }
        // timed from the first movement: bootc can sit a while between its
        // "layers needed" line and the first layer
        if self.began.is_none() && fraction <= from + 1e-9 {
            return Estimate::Calculating;
        }
        let (f_began, began) = *self.began.get_or_insert((fraction, now));
        let elapsed = since(began);
        if elapsed < SEED_SECS {
            // the average since the start: a moving average would still lean
            // on its first, unrepresentative samples
            if elapsed > 0.0 {
                self.rate = Some((fraction - f_began).max(0.0) / elapsed);
            }
        } else if let Some((f0, t0)) = self.last {
            let dt = since(t0);
            if dt > 0.0 {
                let r = (fraction - f0).max(0.0) / dt;
                let a = 1.0 - (-dt / RATE_SECS).exp();
                self.rate = Some(self.rate.map_or(r, |old| old + a * (r - old)));
            }
        }
        self.last = Some((fraction, now));
        match self.rate {
            Some(r) if r > 0.0 && elapsed >= WARMUP_SECS => {
                let rest = (to - fraction) / r;
                // with what came before the UI followed, at today's speed
                let import = (f_began - from) / r + elapsed + rest;
                self.import = Some(import);
                secs(rest + after_import(Some(import)))
            }
            _ => Estimate::Calculating,
        }
    }
}

/// Seconds from the last layer to the end, after an import of `import`
/// seconds (if it was timed).
fn after_import(import: Option<f64>) -> f64 {
    import.map_or(AFTER_SECS, |i| AFTER_BASE_SECS + AFTER_PER_IMPORT * i)
}

fn secs(left: f64) -> Estimate {
    if left > STALLED_SECS {
        // the copy has all but stopped: no number means anything
        Estimate::Calculating
    } else {
        Estimate::Secs(left)
    }
}

/// 186 → "03:06", 3750 → "1:02:30".
fn clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// "layers already present: 0; layers needed: 128 (3.2 GB)" → (0, 128)
fn layer_counts(l: &str) -> Option<(u64, u64)> {
    let present = l
        .strip_prefix("layers already present:")?
        .split(';')
        .next()?
        .trim()
        .parse()
        .ok()?;
    let needed = l
        .split_once("layers needed:")?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((present, needed))
}

/// "layers already present: 0; layers needed: 128 (3.2 GB)" → 3.2
fn layers_gb(l: &str) -> Option<f64> {
    let inner = l.rsplit_once('(')?.1.split_once(')')?.0;
    let mut f = inner.split_whitespace();
    let n: f64 = f.next()?.parse().ok()?;
    let scale = match f.next()? {
        "kB" | "KB" => 1e-6,
        "MB" => 1e-3,
        "GB" => 1.0,
        "TB" => 1e3,
        _ => return None,
    };
    Some(n * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// bootc's output from spike S3 (podman-wrapped, no terminal).
    const S3: &str = "Installing image: docker://ghcr.io/eternalcoder454/atlasos:latest\n\
        Initializing ostree layout\n\
        layers already present: 0; layers needed: 128 (3.2\u{a0}GB)\n\
        Deploying container image...done (18 seconds)\n\
        Bootloader: grub\n\
        Installing bootloader via bootupd\n\
        Executing: \"efibootmgr\" \"--create\"\n\
        BootCurrent: 0002\n\
        Installed: grub.cfg\n\
        Trimming root\n\
        .: 60.8 GiB (65253888000 bytes) trimmed\n\
        Finalizing filesystem root\n\
        Trimming boot\n\
        boot: 1.7 GiB (1776091136 bytes) trimmed\n\
        Finalizing filesystem boot\n\
        Installation complete!\n";

    #[test]
    fn stages_cover_the_bar_without_gaps() {
        let all = [
            Stage::Prepare,
            Stage::Partition,
            Stage::Format,
            Stage::Copy,
            Stage::Deploy,
            Stage::Bootloader,
            Stage::Settings,
            Stage::Finish,
        ];
        assert_eq!(all[0].range().0, 0.0);
        assert_eq!(all[7].range().1, 1.0);
        for w in all.windows(2) {
            assert_eq!(w[0].range().1, w[1].range().0);
        }
    }

    #[test]
    fn s3_output_moves_forward_only() {
        let mut p = BootcProgress::default();
        let mut seen = Vec::new();
        for (i, line) in S3.lines().enumerate() {
            if let Some(x) = p.line(line, Duration::from_secs(i as u64 * 10)) {
                seen.push(x);
            }
        }
        assert!(seen.windows(2).all(|w| w[0].fraction < w[1].fraction));
        assert_eq!(seen.first().unwrap().stage, Stage::Copy);
        let last = seen.last().unwrap();
        assert_eq!(last.stage, Stage::Bootloader);
        assert!((last.fraction - Stage::Bootloader.range().1).abs() < 1e-9);
    }

    #[test]
    fn import_eases_with_time_and_stops_short() {
        let mut p = BootcProgress::default();
        p.line(
            "layers already present: 0; layers needed: 128 (3.2\u{a0}GB)",
            Duration::from_secs(5),
        );
        let at = |p: &mut BootcProgress, s| p.tick(Duration::from_secs(s)).map(|x| x.fraction);
        let a = at(&mut p, 20).unwrap();
        let b = at(&mut p, 80).unwrap();
        let c = at(&mut p, 10_000).unwrap();
        assert!(a < b && b < c);
        assert!(c < Progress::at(Stage::Copy, 0.91).fraction);
        assert_eq!(p.current().stage, Stage::Copy);
        assert_eq!(at(&mut p, 10_001), None, "no change, no signal");
        // a later milestone wins, and the easing stops
        let d = p
            .line(
                "Deploying container image...done (18 seconds)",
                Duration::from_secs(10_002),
            )
            .unwrap();
        assert!(d.fraction > c);
        assert_eq!(d.stage, Stage::Deploy);
        assert_eq!(at(&mut p, 20_000), Some(d.fraction), "only the heartbeat");
    }

    #[test]
    fn counts_move_the_import_and_stop_the_easing() {
        const GB: u64 = 1_000_000_000;
        let s = Duration::from_secs;
        let within = |x: f64| Progress::at(Stage::Copy, x).fraction;
        let mut p = BootcProgress::default();
        // before the import, a count is ignored
        p.line("Installing image: docker://x", s(0));
        assert_eq!(p.counted(Some((GB / 2, GB)), 50, s(0)), None);
        p.line(
            "layers already present: 0; layers needed: 100 (3.0\u{a0}GB)",
            s(1),
        );
        // nothing much copied yet (the proxy's own talk): the easing goes on
        assert_eq!(p.counted(Some((0, GB)), 0, s(2)), None);
        assert_eq!(p.counted(Some((40_000, GB)), 0, s(3)), None);
        assert!(p.tick(s(30)).is_some());
        let eased = p.current().fraction;
        // a real amount takes over; it may sit behind the easing
        assert_eq!(p.counted(Some((2 << 20, GB)), 0, s(31)), None);
        // half the bytes and a tenth of the layers: 30 % of the import
        let x = p.counted(Some((GB / 2, GB)), 10, s(32)).unwrap().fraction;
        assert!((x - within(0.06 + 0.94 * 0.3)).abs() < 1e-9 && x > eased);
        // time alone no longer moves it, past the heartbeat
        assert_eq!(p.tick(s(33)), None);
        assert_eq!(p.tick(s(37)).unwrap().fraction, x);
        // more bytes than the image: short of the end while layers remain
        let y = p.counted(Some((2 * GB, GB)), 80, s(40)).unwrap().fraction;
        assert!((y - within(0.06 + 0.94 * 0.9)).abs() < 1e-9);
        // all in: the end of the copy, then the silent deploy eases on
        let deploy = |x: f64| Progress::at(Stage::Deploy, x).fraction;
        let end = p.counted(Some((GB, GB)), 100, s(50)).unwrap();
        assert_eq!(end.stage, Stage::Deploy);
        assert!((end.fraction - within(IMPORT_TO)).abs() < 1e-9);
        assert!((end.fraction - deploy(0.0)).abs() < 1e-9);
        assert_eq!(p.counted(Some((GB, GB)), 100, s(52)), None);
        let a = p.tick(s(110)).unwrap().fraction;
        let b = p.tick(s(10_000)).unwrap().fraction;
        assert!(end.fraction < a && a < b && b < deploy(0.99) + 1e-9);
        // the deploy line, then nothing but the line and heartbeat matter
        let d = p
            .line("Deploying container image...done (87 seconds)", s(10_001))
            .unwrap();
        assert!((d.fraction - deploy(1.0)).abs() < 1e-9);
        assert_eq!(p.counted(Some((5 * GB, GB)), 200, s(10_002)), None);
        assert_eq!(p.tick(s(10_003)), None);
    }

    #[test]
    fn the_layers_decide_when_the_copy_is_in() {
        const GB: u64 = 1_000_000_000;
        let s = Duration::from_secs;
        let to = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        let start = |p: &mut BootcProgress| {
            p.line(
                "layers already present: 0; layers needed: 127 (3.0\u{a0}GB)",
                s(0),
            );
        };
        // all the bytes, one layer still importing: not yet
        let mut p = BootcProgress::default();
        start(&mut p);
        p.counted(Some((GB, GB)), 126, s(10));
        assert!(p.current().fraction < to);
        p.counted(Some((GB, GB)), 126, s(30));
        assert!(p.current().fraction < to);
        // the last layer
        p.counted(Some((GB, GB)), 127, s(32));
        assert!((p.current().fraction - to).abs() < 1e-9);
        // a podman size too large: the layers are all in all the same
        let mut p = BootcProgress::default();
        start(&mut p);
        p.counted(Some((GB / 2, GB)), 127, s(10));
        assert!((p.current().fraction - to).abs() < 1e-9);
        // a layer bootc lists twice, with one ref: the counts stand still
        // near the end, and after a while that is the end
        let mut p = BootcProgress::default();
        start(&mut p);
        p.counted(Some((GB, GB)), 126, s(10));
        p.counted(Some((GB, GB)), 126, s(38));
        assert!(p.current().fraction < to);
        p.counted(Some((GB, GB)), 126, s(40));
        assert!((p.current().fraction - to).abs() < 1e-9);
        // standing still with layers left is a slow copy, not the end
        let mut p = BootcProgress::default();
        start(&mut p);
        p.counted(Some((GB / 2, GB)), 60, s(10));
        p.counted(Some((GB / 2, GB)), 60, s(100));
        assert!(p.current().fraction < to);
        let mut p = BootcProgress::default();
        start(&mut p);
        p.counted(Some((GB, GB)), 121, s(10));
        p.counted(Some((GB, GB)), 121, s(100));
        assert!(p.current().fraction < to);
        // without a layer count, near the end and still: the end
        let mut p = BootcProgress::default();
        p.line("layers already present: some", s(0));
        p.counted(Some((GB * 96 / 100, GB)), 0, s(10));
        p.counted(Some((GB * 96 / 100, GB)), 0, s(38));
        assert!(p.current().fraction < to);
        p.counted(Some((GB * 96 / 100, GB)), 0, s(40));
        assert!((p.current().fraction - to).abs() < 1e-9);
    }

    #[test]
    fn either_count_alone_moves_the_import() {
        const GB: u64 = 1_000_000_000;
        let s = Duration::from_secs;
        let within = |x: f64| Progress::at(Stage::Copy, x).fraction;
        // no image size: layers alone
        let mut p = BootcProgress::default();
        p.line("layers already present: 2; layers needed: 10 (1 GB)", s(0));
        assert_eq!(p.counted(None, 2, s(1)), None, "only the layers present");
        let x = p.counted(None, 7, s(2)).unwrap().fraction;
        assert!((x - within(0.06 + 0.94 * 0.5)).abs() < 1e-9);
        // no layer count in bootc's line: bytes alone
        let mut p = BootcProgress::default();
        p.line("layers already present: some", s(0));
        let x = p.counted(Some((GB / 4, GB)), 3, s(1)).unwrap().fraction;
        assert!((x - within(0.06 + 0.94 * 0.25)).abs() < 1e-9);
        // neither: the easing goes on
        let mut p = BootcProgress::default();
        p.line("layers already present: some", s(0));
        assert_eq!(p.counted(None, 3, s(1)), None);
        assert!(p.tick(s(30)).is_some());
    }

    /// Feeds `t` the bar of a copy, sampled every two seconds as the helper
    /// sends it, and gives the text at the end.
    fn follow(t: &mut TimeLeft, from: u64, to: u64, bar: impl Fn(u64) -> f64) -> String {
        let mut text = String::new();
        let mut s = from;
        while s <= to {
            text = t.text(bar(s), Duration::from_secs(s));
            s += 2;
        }
        text
    }

    /// "03:06 left" → 186
    fn left(text: &str) -> Option<i64> {
        let clock = text.strip_suffix(" left")?;
        clock
            .split(':')
            .try_fold(0, |a, n| Some(a * 60 + n.parse::<i64>().ok()?))
    }

    /// Seconds from the last layer to the end, after `import` seconds of it.
    fn after(import: f64) -> i64 {
        after_import(Some(import)).round() as i64
    }

    /// Asserts `text` shows `secs` left, give or take `slack`.
    #[track_caller]
    fn about(text: &str, secs: i64, slack: i64) {
        let n = left(text).unwrap_or_else(|| panic!("no time in {text:?}"));
        assert!((n - secs).abs() <= slack, "{text}, not {secs} s");
    }

    #[test]
    fn the_clock() {
        assert_eq!(clock(0), "00:00");
        assert_eq!(clock(59), "00:59");
        assert_eq!(clock(186), "03:06");
        assert_eq!(clock(3599), "59:59");
        assert_eq!(clock(3750), "1:02:30");
        assert_eq!(left("1:02:30 left"), Some(3750));
    }

    #[test]
    fn time_left_follows_a_slow_copy() {
        // the VM install that sat on "About 2 minutes left": 1 min 43 s to
        // format, then 730 s of import, then the deploy
        let a = Progress::at(Stage::Copy, IMPORT_FROM).fraction;
        let b = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        let bar = |s: u64| match s {
            0..103 => Progress::at(Stage::Format, s as f64 / 103.0).fraction,
            103..833 => a + (b - a) * (s - 103) as f64 / 730.0,
            _ => Progress::at(Stage::Deploy, 0.5).fraction,
        };
        let after = after(730.0);
        let mut t = TimeLeft::default();
        assert_eq!(t.text(0.0, Duration::ZERO), "Calculating the time left…");
        assert_eq!(follow(&mut t, 0, 100, bar), "Calculating the time left…");
        // the speed isn't believed in the first seconds
        assert_eq!(follow(&mut t, 102, 112, bar), "Calculating the time left…");
        // then, all the way: what is left of the import, and the time after
        let mut s = 120;
        while s < 834 {
            about(
                &t.text(bar(s), Duration::from_secs(s)),
                833 - s as i64 + after,
                2,
            );
            // the clock runs down between samples
            about(
                &t.tick(Duration::from_secs(s + 1)),
                833 - s as i64 + after - 1,
                2,
            );
            s += 2;
        }
        // the deploy: the time after the import runs out
        about(&follow(&mut t, 834, 900, bar), after - 66, 2);
        // it took longer: "Almost done", then bootc's line starts the last
        // few seconds
        assert_eq!(
            t.tick(Duration::from_secs(833 + after as u64 + 2)),
            "Almost done"
        );
        let deployed = Progress::at(Stage::Bootloader, 0.0).fraction;
        about(
            &t.text(deployed, Duration::from_secs(1000)),
            POST_SECS as i64,
            1,
        );
        about(&t.tick(Duration::from_secs(1004)), POST_SECS as i64 - 4, 1);
        assert_eq!(
            t.text(Stage::Finish.range().0, Duration::from_secs(1010)),
            "Almost done"
        );
    }

    #[test]
    fn a_slow_start_does_not_inflate_the_estimate() {
        let a = Progress::at(Stage::Copy, IMPORT_FROM).fraction;
        let b = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        // the "layers needed" line, 10 s of nothing, then 600 s of import
        let at = |s: u64| a + (b - a) * (s.saturating_sub(10) as f64 / 600.0).min(1.0);
        let mut t = TimeLeft::default();
        assert_eq!(follow(&mut t, 0, 24, at), "Calculating the time left…");
        // 610 - 28 s of import, and the time after 600 s of it
        about(&follow(&mut t, 26, 28, at), 610 - 28 + after(600.0), 2);
    }

    #[test]
    fn time_left_goes_up_when_the_copy_slows() {
        let a = Progress::at(Stage::Copy, IMPORT_FROM).fraction;
        let b = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        // 300 s of import at the first speed, a quarter of it after 120 s
        let at = |s: u64| {
            let done = if s <= 120 {
                s as f64 / 300.0
            } else {
                0.4 + (s - 120) as f64 / 1200.0
            };
            a + (b - a) * done.min(1.0)
        };
        let mut t = TimeLeft::default();
        // 180 s of import left, and the time after 300 s of it
        let first = follow(&mut t, 0, 120, at);
        about(&first, 180 + after(300.0), 2);
        // two minutes at the new speed, with less left to do: the average
        // has moved toward it, and the clock with it
        let later = follow(&mut t, 122, 240, at);
        assert!(left(&later) > left(&first), "{first}, then {later}");
    }

    /// The copy in the VM (7.10 GB, 127 layers), every 30 s: GB handed to
    /// bootc, and layers imported. It runs in bursts.
    const VM_GB: [f64; 37] = [
        0.03, 0.38, 0.43, 0.45, 0.47, 0.49, 0.56, 0.74, 0.79, 1.18, 1.30, 1.30, 1.42, 1.55, 1.77,
        2.19, 2.62, 2.81, 2.95, 3.16, 3.26, 3.38, 3.91, 4.11, 4.33, 4.49, 4.64, 5.15, 5.25, 5.41,
        5.50, 5.64, 5.75, 6.06, 6.79, 7.07, 7.10,
    ];
    const VM_LAYERS: [u64; 37] = [
        0, 4, 6, 7, 9, 10, 14, 22, 24, 30, 32, 32, 38, 45, 51, 53, 57, 63, 68, 72, 76, 80, 84, 88,
        91, 94, 97, 100, 103, 106, 108, 111, 113, 116, 123, 126, 127,
    ];

    #[test]
    fn the_vm_copy_gets_a_steady_estimate() {
        let s = Duration::from_secs;
        let mut p = BootcProgress::default();
        p.line(
            "layers already present: 0; layers needed: 127 (3.0\u{a0}GB)",
            s(0),
        );
        let mut t = TimeLeft::default();
        // the last layer at 1080 s, and the time after it
        let end = 1080 + after(1080.0);
        let mut before: Option<i64> = None;
        let mut rises = 0;
        for now in (0..1080).step_by(2) {
            let (i, x) = (now / 30, (now % 30) as f64 / 30.0);
            let gb = VM_GB[i] + (VM_GB[i + 1] - VM_GB[i]) * x;
            let layers = VM_LAYERS[i] + ((VM_LAYERS[i + 1] - VM_LAYERS[i]) as f64 * x) as u64;
            let bytes = Some(((gb * 1e9) as u64, 7_100_000_000));
            p.counted(bytes, layers, s(now as u64))
                .or_else(|| p.tick(s(now as u64)));
            let text = t.text(p.current().fraction, s(now as u64));
            if now < 60 {
                continue;
            }
            let n = left(&text).unwrap_or_else(|| panic!("{now} s: {text}"));
            // a 30 s average showed from 7 minutes to over 2 hours here; the
            // slow first five minutes still make it high for a while
            let off = n - (end - now as i64);
            assert!(
                (-270..=360).contains(&off),
                "{now} s: {text}, off by {off} s"
            );
            // (the time after the import grows with it, so a high estimate
            // of the import is a little higher still)
            if now >= 420 {
                assert!(off.abs() <= 130, "{now} s: {text}, off by {off} s");
            }
            // a clock, not a jumping number: it moves a few seconds at a
            // time, and once the copy has settled, mostly down
            if let Some(b) = before {
                assert!((-14..=12).contains(&(n - b)), "{now} s: from {b} to {n} s");
                if now >= 240 && n > b {
                    rises += 1;
                }
            }
            before = Some(n);
        }
        assert!(rises < 30, "{rises} of 420 steps went up");
        // the last layer, then the silent merge and deploy, toward bootc's line
        p.counted(Some((7_100_000_000, 7_100_000_000)), 127, s(1080));
        assert!(p.current().fraction >= Progress::at(Stage::Copy, IMPORT_TO).fraction - 1e-9);
        let mut text = String::new();
        for now in (1080..=1140).step_by(2) {
            p.tick(s(now));
            text = t.text(p.current().fraction, s(now));
        }
        assert_eq!(p.current().stage, Stage::Deploy);
        // the time after the last layer, 60 s of it gone
        about(&text, after(1080.0) - 60, 10);
    }

    /// The timed install on real disks: bootc's "layers needed" line at
    /// 24 s, the copy slow, then fast, then in between (8, 20 and 11 MB/s,
    /// 7.25 GB, 127 layers), the last layer at 570 s, the deploy done at
    /// 691 s and the install at 695 s. Gives the seconds shown and the
    /// seconds truly left, every two seconds from a minute in.
    fn real_install() -> Vec<(u64, i64, i64)> {
        let s = Duration::from_secs;
        let total = 7248;
        let mb = |t: u64| match t {
            ..24 => 0,
            24..150 => (t - 24) * 8,
            150..330 => 1008 + (t - 150) * 20,
            _ => (4608 + (t - 330) * 11).min(total),
        };
        let mut p = BootcProgress::default();
        let mut t = TimeLeft::default();
        let mut seen = Vec::new();
        for now in (0..=694).step_by(2) {
            if now == 24 {
                p.line(
                    "layers already present: 0; layers needed: 127 (7.25\u{a0}GB)",
                    s(now),
                );
            }
            if now == 692 {
                p.line("Deploying container image...done (52 seconds)", s(now));
                p.line("Installing bootloader via bootupd", s(now));
            }
            let m = mb(now);
            let bytes = Some((m * 1_000_000, total * 1_000_000));
            p.counted(bytes, m * 127 / total, s(now))
                .or_else(|| p.tick(s(now)));
            let text = t.text(p.current().fraction, s(now));
            if now >= 60 {
                let n = left(&text).unwrap_or_else(|| panic!("{now} s: {text}"));
                seen.push((now, n, 695 - now as i64));
            }
        }
        seen
    }

    #[test]
    fn a_real_install_counts_down_to_the_end() {
        let seen = real_install();
        for w in seen.windows(2) {
            let ((_, b, _), (now, n, _)) = (w[0], w[1]);
            assert!((-14..=12).contains(&(n - b)), "{now} s: from {b} to {n} s");
        }
        for &(now, n, truth) in &seen {
            let off = n - truth;
            let most = match now {
                // the slow start can't tell of the fast middle
                ..240 => 450,
                // the speed more than halves at 330 s: the clock follows
                240..572 => 100,
                // after the last layer: the time after it, counted down
                _ => 15,
            };
            assert!(off.abs() <= most, "{now} s: {n} s left, off by {off} s");
        }
    }

    #[test]
    fn a_ui_that_follows_from_the_deploy_on_counts_what_is_left() {
        let s = Duration::from_secs;
        // 60 s into the deploy, eased as BootcProgress does
        let mut p = BootcProgress::default();
        p.line("layers already present: 0; layers needed: 2", s(0));
        p.counted(None, 2, s(100));
        p.tick(s(160));
        assert_eq!(p.current().stage, Stage::Deploy);
        let mut t = TimeLeft::default();
        about(
            &t.text(p.current().fraction, s(5)),
            AFTER_SECS as i64 - 60,
            1,
        );
        // the UI restarted after the deploy: the last few seconds
        let mut t = TimeLeft::default();
        let deployed = Progress::at(Stage::Bootloader, 0.0).fraction;
        about(&t.text(deployed, s(5)), POST_SECS as i64, 1);
        about(&t.tick(s(8)), POST_SECS as i64 - 3, 1);
    }

    #[test]
    fn the_deploy_line_before_the_last_count_moves_on() {
        let s = Duration::from_secs;
        let mut p = BootcProgress::default();
        p.line("layers already present: 0; layers needed: 100", s(0));
        let a = p.counted(None, 60, s(10)).unwrap();
        let d = p
            .line("Deploying container image...done (52 seconds)", s(20))
            .unwrap();
        assert_eq!(d.stage, Stage::Deploy);
        assert!(a.fraction < d.fraction);
        // the counts that come late change nothing
        assert_eq!(p.counted(None, 100, s(22)), None);
        assert_eq!(p.tick(s(23)), None);
        let b = p.line("Installing bootloader via bootupd", s(24)).unwrap();
        assert!(d.fraction < b.fraction);
    }

    #[test]
    fn a_stalled_copy_shows_no_number() {
        let a = Progress::at(Stage::Copy, IMPORT_FROM).fraction;
        let b = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        let at = |s: u64| a + (b - a) * (s.min(60) as f64 / 600.0);
        let mut t = TimeLeft::default();
        about(&follow(&mut t, 0, 60, at), 540 + after(600.0), 2);
        // the average falls by e every 3 minutes: past 3 hours after 9
        assert_ne!(follow(&mut t, 62, 500, at), "Calculating the time left…");
        assert_eq!(follow(&mut t, 502, 700, at), "Calculating the time left…");
        assert_eq!(
            t.tick(Duration::from_secs(701)),
            "Calculating the time left…"
        );
        // it moves again
        let mut s = 700;
        let again = |x: u64| a + (b - a) * ((60 + x.saturating_sub(700)) as f64 / 600.0);
        while s < 820 {
            t.text(again(s), Duration::from_secs(s));
            s += 2;
        }
        assert!(left(&t.text(again(820), Duration::from_secs(820))).is_some());
    }

    #[test]
    fn a_clock_that_ran_out_stays_almost_done() {
        let a = Progress::at(Stage::Copy, IMPORT_FROM).fraction;
        let b = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        let at = |s: u64| a + (b - a) * (s.min(600) as f64 / 600.0);
        let mut t = TimeLeft::default();
        about(&follow(&mut t, 0, 100, at), 500 + after(600.0), 2);
        // the clock runs out, and a later estimate just past it doesn't
        // bring it back for a moment
        t.finish = Some(110.0);
        assert_eq!(t.tick(Duration::from_secs(111)), "Almost done");
        t.finish = Some(131.0);
        assert_eq!(t.tick(Duration::from_secs(112)), "Almost done");
        // a real number does
        assert!(left(&t.text(at(114), Duration::from_secs(114))).is_some());
    }

    #[test]
    fn a_ui_that_follows_late_learns_the_speed_again() {
        let a = Progress::at(Stage::Copy, IMPORT_FROM).fraction;
        let b = Progress::at(Stage::Copy, IMPORT_TO).fraction;
        // the UI restarted 400 s into a 600 s import: its clock starts at 0
        let at = |s: u64| a + (b - a) * ((s + 400).min(600) as f64 / 600.0);
        let mut t = TimeLeft::default();
        assert_eq!(t.text(at(0), Duration::ZERO), "Calculating the time left…");
        // 200 s - 20 s of import, and the time after all 600 s of it
        about(&follow(&mut t, 2, 20, at), 180 + after(600.0), 2);
    }

    #[test]
    fn layer_sizes() {
        assert_eq!(
            layers_gb("layers already present: 0; layers needed: 128 (3.2\u{a0}GB)"),
            Some(3.2)
        );
        assert_eq!(
            layers_gb("layers already present: 1; layers needed: 2 (512 MB)"),
            Some(0.512)
        );
        assert_eq!(layers_gb("layers needed: 2"), None);
        assert_eq!(
            layer_counts("layers already present: 0; layers needed: 127 (3.0\u{a0}GB)"),
            Some((0, 127))
        );
        assert_eq!(
            layer_counts("layers already present: 3; layers needed: 2"),
            Some((3, 2))
        );
        assert_eq!(
            layer_counts("layers already present: x; layers needed: 2"),
            None
        );
        assert_eq!(layer_counts("layers already present: 1"), None);
    }
}
