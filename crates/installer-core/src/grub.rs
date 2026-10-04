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

/// A GRUB password hash as `grub2-mkpasswd-pbkdf2` prints it:
/// `grub.pbkdf2.sha512.<iterations>.<hex salt>.<hex hash>`.
pub fn validate_pbkdf2(h: &str) -> Result<(), String> {
    let f: Vec<&str> = h.split('.').collect();
    let hex = |s: &str| {
        !s.is_empty() && s.len().is_multiple_of(2) && s.bytes().all(|b| b.is_ascii_hexdigit())
    };
    let ok = f.len() == 6
        && f[..3] == ["grub", "pbkdf2", "sha512"]
        && !f[3].is_empty()
        && f[3].len() <= 9
        && f[3].bytes().all(|b| b.is_ascii_digit())
        && f[3].parse::<u32>().is_ok_and(|n| n >= 1000)
        && hex(f[4])
        && hex(f[5]);
    if ok {
        Ok(())
    } else {
        Err("not a GRUB password hash".into())
    }
}

/// The hash from `grub2-mkpasswd-pbkdf2`'s output: after its two prompts, the
/// text `PBKDF2 hash of your password is <hash>`. The prompts go to stdout
/// too, and with a pipe for stdin they aren't followed by a newline, so the
/// marker may follow them on the same line.
pub fn parse_mkpasswd(stdout: &str) -> Result<&str, String> {
    const MARKER: &str = "PBKDF2 hash of your password is ";
    let hash = stdout
        .lines()
        .filter_map(|l| l.split_once(MARKER).map(|(_, h)| h.trim()))
        .collect::<Vec<_>>();
    match hash.as_slice() {
        [h] => validate_pbkdf2(h).map(|()| *h),
        _ => Err("grub2-mkpasswd-pbkdf2 gave no password hash".into()),
    }
}

/// `/boot/grub2/user.cfg`: bootupd's static config makes `root` the only
/// superuser when `GRUB2_PASSWORD` is set, so the menu entries can't be
/// edited and there is no GRUB command line, while the boot entries stay
/// bootable without the password.
pub fn user_cfg(hash: &str) -> Result<String, String> {
    validate_pbkdf2(hash)?;
    Ok(format!("GRUB2_PASSWORD={hash}\n"))
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
            "\nmenuentry '{title}' --class windows --class os --unrestricted {{\n\
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
        assert!(c.contains("menuentry 'Windows' --class windows --class os --unrestricted {\n"));
        assert!(c.contains("\tsearch --no-floppy --fs-uuid --set=root 4A1B-2C3D\n"));
        assert!(c.contains("\tchainloader /EFI/Microsoft/Boot/bootmgfw.efi\n}\n"));
    }

    const HASH: &str = "grub.pbkdf2.sha512.10000.A81F1971A0042577.12F7519BDEA05552";

    #[test]
    fn windows_stays_bootable_when_grub_is_locked() {
        let c = custom_cfg(&["4A1B-2C3D".into(), "1234-ABCD".into()]).unwrap();
        assert_eq!(c.matches("--unrestricted {").count(), 2);
    }

    #[test]
    fn grub_password_hashes_are_strict() {
        let out = format!(
            "Enter password: \nReenter password: \nPBKDF2 hash of your password is {HASH}\n"
        );
        assert_eq!(parse_mkpasswd(&out), Ok(HASH));
        // the prompts and the hash on one line, as with a pipe
        let out =
            format!("Enter password: Reenter password: PBKDF2 hash of your password is {HASH}\n");
        assert_eq!(parse_mkpasswd(&out), Ok(HASH));
        assert_eq!(user_cfg(HASH).unwrap(), format!("GRUB2_PASSWORD={HASH}\n"));
        for bad in [
            "",
            "Enter password: \n",
            &format!(
                "PBKDF2 hash of your password is {HASH}\nPBKDF2 hash of your password is {HASH}\n"
            ),
            "PBKDF2 hash of your password is grub.pbkdf2.sha512.10000.AB.XYZ",
            "PBKDF2 hash of your password is grub.pbkdf2.sha512.10.AB.CD",
            "PBKDF2 hash of your password is grub.pbkdf2.sha256.10000.AB.CD",
            "PBKDF2 hash of your password is grub.pbkdf2.sha512.10000.ABC.CD",
            "PBKDF2 hash of your password is grub.pbkdf2.sha512.10000.AB.CD.EF",
        ] {
            assert!(parse_mkpasswd(bad).is_err(), "{bad}");
        }
        for bad in [
            "",
            "x\nGRUB2_PASSWORD=y",
            "grub.pbkdf2.sha512.10000.AB.CD\n#",
        ] {
            assert!(user_cfg(bad).is_err(), "{bad:?}");
        }
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
