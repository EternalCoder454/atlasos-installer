//! `/boot/grub2/custom.cfg`: Windows in the boot menu. bootupd's static
//! grub.cfg sources it (`41_custom.cfg`) after its own settings, and bootupd
//! has no os-prober, so without it Windows is not in the menu at all.

/// A FAT volume ID as blkid prints it, `ABCD-1234`.
pub fn validate_fat_uuid(s: &str) -> Result<(), String> {
    let ok = s.len() == 9
        && s.bytes().enumerate().all(|(i, b)| {
            if i == 4 {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        });
    if ok {
        Ok(())
    } else {
        Err(format!("not a FAT volume ID: {s:?}"))
    }
}

/// The file for the EFI partitions (by FAT volume ID) that hold Windows Boot
/// Manager. AtlasOS shows the menu for 1 s (`grub-static-pre.cfg`); with
/// Windows beside it the menu stays 5 s, so Windows is easy to reach.
pub fn custom_cfg(windows_esps: &[String]) -> Result<String, String> {
    let mut s = String::from(
        "# Written by Atlas Installer: Windows in the boot menu.\n\
         set timeout=5\n\
         set timeout_style=menu\n",
    );
    for (i, uuid) in windows_esps.iter().enumerate() {
        validate_fat_uuid(uuid)?;
        let title = if windows_esps.len() == 1 {
            "Windows".to_string()
        } else {
            format!("Windows ({})", i + 1)
        };
        s.push_str(&format!(
            "\nmenuentry '{title}' --class windows --class os {{\n\
             \tinsmod part_gpt\n\
             \tinsmod fat\n\
             \tsearch --no-floppy --fs-uuid --set=root {uuid}\n\
             \tchainloader /EFI/Microsoft/Boot/bootmgfw.efi\n\
             }}\n"
        ));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_windows() {
        let c = custom_cfg(&["4A1B-2C3D".into()]).unwrap();
        assert!(c.contains("set timeout=5\n"));
        assert!(c.contains("menuentry 'Windows' --class windows --class os {\n"));
        assert!(c.contains("\tsearch --no-floppy --fs-uuid --set=root 4A1B-2C3D\n"));
        assert!(c.contains("\tchainloader /EFI/Microsoft/Boot/bootmgfw.efi\n}\n"));
    }

    #[test]
    fn two_windows_are_numbered() {
        let c = custom_cfg(&["4A1B-2C3D".into(), "1234-ABCD".into()]).unwrap();
        assert!(c.contains("'Windows (1)'") && c.contains("'Windows (2)'"));
    }

    #[test]
    fn junk_ids_are_refused() {
        for bad in [
            "",
            "4A1B2C3D",
            "4A1B-2C3D\n",
            "'; reboot",
            "4A1B-2C3Z",
            "0b4f6b8e-2a0c-4d5e-9f1a-3c2b1a0d9e8f",
        ] {
            assert!(custom_cfg(&[bad.into()]).is_err(), "{bad}");
        }
    }
}
