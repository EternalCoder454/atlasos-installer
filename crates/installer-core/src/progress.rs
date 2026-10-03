//! Install progress, weighted by stage. bootc install has no progress
//! stream, and without a terminal it prints only a few milestone lines
//! (captured in spike S3 and checked in the VM), so the copy stage moves on
//! those and on elapsed time in between.

use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Prepare,
    Partition,
    Format,
    Copy,
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
            Stage::Copy => (0.07, 0.85),
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

/// Follows `bootc install to-filesystem`'s output. Never goes backwards.
#[derive(Debug, Clone)]
pub struct BootcProgress {
    last: Progress,
    /// When the layer import started, and how long it is expected to take.
    import: Option<(Duration, Duration)>,
}

/// Seconds of import per GB, measured in the VM (3.2 GB in about 75 s); a
/// real NVMe disk is faster, so the bar then jumps ahead at the next line.
const SECS_PER_GB: f64 = 24.0;

impl Default for BootcProgress {
    fn default() -> Self {
        BootcProgress {
            last: Progress::at(Stage::Copy, 0.0),
            import: None,
        }
    }
}

impl BootcProgress {
    pub fn current(&self) -> &Progress {
        &self.last
    }

    fn advance(&mut self, p: Progress) -> Option<Progress> {
        if p.fraction > self.last.fraction + 1e-4 || p.stage > self.last.stage {
            if p.fraction >= self.last.fraction {
                self.last = p;
            } else {
                self.last.stage = p.stage;
            }
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
            Progress::at(Stage::Copy, 0.06)
        } else if l.starts_with("Deploying container image") {
            self.import = None;
            Progress::at(Stage::Copy, if l.contains("done") { 1.0 } else { 0.92 })
        } else if l.starts_with("Bootloader:") || l.starts_with("Installing bootloader") {
            self.import = None;
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
        self.advance(p)
    }

    /// Time passes during the layer import: ease toward 90 % of the stage.
    pub fn tick(&mut self, now: Duration) -> Option<Progress> {
        let (start, expect) = self.import?;
        let t = now.saturating_sub(start).as_secs_f64() / expect.as_secs_f64();
        // 1 - e^-t: 63 % of the way at the expected time, never arriving
        let within = 0.06 + (0.90 - 0.06) * (1.0 - (-t).exp());
        self.advance(Progress::at(Stage::Copy, within))
    }
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
            Stage::Bootloader,
            Stage::Settings,
            Stage::Finish,
        ];
        assert_eq!(all[0].range().0, 0.0);
        assert_eq!(all[6].range().1, 1.0);
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
        assert_eq!(at(&mut p, 10_001), None, "no change, no signal");
        // a later milestone wins, and ticks stop
        let d = p
            .line(
                "Deploying container image...done (18 seconds)",
                Duration::from_secs(10_002),
            )
            .unwrap();
        assert!(d.fraction > c);
        assert_eq!(at(&mut p, 20_000), None);
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
    }
}
