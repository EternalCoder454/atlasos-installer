//! `lsblk --json` output, as the helper runs it ([`ARGS`]).

use serde::{Deserialize, Deserializer};

/// The lsblk arguments the parser expects: a tree, sizes in bytes, full
/// device paths.
pub const ARGS: [&str; 5] = [
    "--json",
    "--bytes",
    "--paths",
    "--output",
    "NAME,KNAME,TYPE,SIZE,RO,RM,HOTPLUG,TRAN,MODEL,VENDOR,LABEL,PARTLABEL,PARTTYPE,PARTUUID,UUID,FSTYPE,MOUNTPOINTS,PTTYPE,PTUUID,SERIAL,WWN,LOG-SEC",
];

#[derive(Debug, Clone, Deserialize)]
pub struct Lsblk {
    pub blockdevices: Vec<Device>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Device {
    /// Full path, e.g. `/dev/nvme0n1p2` or `/dev/mapper/ventoy`.
    pub name: String,
    /// Kernel path, e.g. `/dev/dm-0`.
    #[serde(default)]
    pub kname: Option<String>,
    /// disk, part, rom, loop, lvm, crypt, raid1, mpath, ...
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, deserialize_with = "number")]
    pub size: u64,
    #[serde(default, deserialize_with = "flag")]
    pub ro: bool,
    #[serde(default, deserialize_with = "flag")]
    pub rm: bool,
    #[serde(default, deserialize_with = "flag")]
    pub hotplug: bool,
    #[serde(default)]
    pub tran: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub vendor: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub partlabel: Option<String>,
    #[serde(default)]
    pub parttype: Option<String>,
    #[serde(default)]
    pub partuuid: Option<String>,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub fstype: Option<String>,
    #[serde(default)]
    pub mountpoints: Vec<Option<String>>,
    #[serde(default)]
    pub pttype: Option<String>,
    #[serde(default)]
    pub ptuuid: Option<String>,
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub wwn: Option<String>,
    #[serde(rename = "log-sec", default, deserialize_with = "opt_number")]
    pub log_sec: Option<u64>,
    #[serde(default)]
    pub children: Vec<Device>,
}

impl Lsblk {
    pub fn parse(json: &str) -> Result<Lsblk, String> {
        serde_json::from_str(json).map_err(|e| format!("cannot read lsblk output: {e}"))
    }

    /// The top-level device with this path.
    pub fn find(&self, path: &str) -> Option<&Device> {
        self.blockdevices.iter().find(|d| d.name == path)
    }
}

impl Device {
    /// This device and everything under it (partitions, device-mapper maps).
    pub fn walk(&self) -> Vec<&Device> {
        let mut out = vec![self];
        let mut i = 0;
        while i < out.len() {
            out.extend(out[i].children.iter());
            i += 1;
        }
        out
    }

    /// The kernel name without `/dev/`, e.g. `nvme0n1`.
    pub fn id(&self) -> &str {
        self.name.strip_prefix("/dev/").unwrap_or(&self.name)
    }

    /// Where it is mounted (`[SWAP]` for active swap).
    pub fn mounts(&self) -> impl Iterator<Item = &str> {
        self.mountpoints.iter().flatten().map(String::as_str)
    }

    /// Its partitions (direct children of type `part`).
    pub fn partitions(&self) -> impl Iterator<Item = &Device> {
        self.children.iter().filter(|c| c.kind == "part")
    }

    pub fn sector_size(&self) -> u64 {
        self.log_sec.filter(|s| *s >= 512).unwrap_or(512)
    }

    fn text(v: &Option<String>) -> Option<&str> {
        v.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }

    pub fn fstype(&self) -> Option<&str> {
        Self::text(&self.fstype)
    }

    pub fn label(&self) -> Option<&str> {
        Self::text(&self.label)
    }

    pub fn partlabel(&self) -> Option<&str> {
        Self::text(&self.partlabel)
    }

    pub fn model(&self) -> Option<&str> {
        Self::text(&self.model)
    }

    pub fn vendor(&self) -> Option<&str> {
        Self::text(&self.vendor)
    }

    pub fn tran(&self) -> Option<&str> {
        Self::text(&self.tran)
    }

    pub fn serial(&self) -> Option<&str> {
        Self::text(&self.serial)
    }
}

/// lsblk prints numbers and booleans as JSON values in util-linux 2.37 and
/// later, and as strings before.
fn number<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    opt_number(d).map(|n| n.unwrap_or(0))
}

fn opt_number<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| D::Error::custom("bad number")),
        serde_json::Value::String(s) => s.trim().parse().map(Some).map_err(D::Error::custom),
        _ => Err(D::Error::custom("expected a number")),
    }
}

fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    use serde::de::Error;
    match serde_json::Value::deserialize(d)? {
        serde_json::Value::Null => Ok(false),
        serde_json::Value::Bool(b) => Ok(b),
        serde_json::Value::Number(n) => Ok(n.as_u64() != Some(0)),
        serde_json::Value::String(s) => Ok(s.trim() == "1"),
        _ => Err(D::Error::custom("expected a flag")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_style_strings_and_new_style_values_both_parse() {
        let old = r#"{"blockdevices":[{"name":"/dev/sda","type":"disk","size":"1000","ro":"0","rm":"1","log-sec":"4096"}]}"#;
        let new = r#"{"blockdevices":[{"name":"/dev/sda","type":"disk","size":1000,"ro":false,"rm":true,"log-sec":4096,"mountpoints":[null]}]}"#;
        for j in [old, new] {
            let l = Lsblk::parse(j).unwrap();
            let d = &l.blockdevices[0];
            assert_eq!(
                (d.size, d.ro, d.rm, d.sector_size()),
                (1000, false, true, 4096)
            );
            assert_eq!(d.mounts().count(), 0);
            assert_eq!(d.id(), "sda");
        }
    }

    #[test]
    fn walk_visits_every_level() {
        let j = r#"{"blockdevices":[{"name":"/dev/sda","type":"disk","children":[
            {"name":"/dev/sda1","type":"part","children":[{"name":"/dev/mapper/x","type":"crypt"}]},
            {"name":"/dev/sda2","type":"part"}]}]}"#;
        let l = Lsblk::parse(j).unwrap();
        let names: Vec<_> = l.blockdevices[0]
            .walk()
            .iter()
            .map(|d| d.name.clone())
            .collect();
        assert_eq!(
            names,
            ["/dev/sda", "/dev/sda1", "/dev/sda2", "/dev/mapper/x"]
        );
        assert_eq!(l.blockdevices[0].partitions().count(), 2);
    }
}
