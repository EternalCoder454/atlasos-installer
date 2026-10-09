//! Whether the live system's own container policy proves that the installed
//! system's first update will be checked against Telamon OS's signature, so
//! the install can ask bootc to record that (`--enforce-container-sigpolicy`:
//! the new system's origin becomes `ostree-image-signed:docker://...`).
//!
//! That flag makes bootc read `containers-policy.json` and refuse a default
//! of `insecureAcceptAnything`; with the flag, a policy that cannot verify the
//! image would make every later pull fail. So the flag is only passed when
//! the policy has what the image build puts there (`build_files/build.sh` of
//! Telamon OS): a default that accepts nothing unchecked, and for the image's
//! repository a requirement of `sigstoreSigned` with keys that exist, and
//! the registry configuration that tells containers/image to look for the
//! signatures. No I/O here; the helper reads the files.

/// The repository of an image reference: no tag, no digest.
pub fn repository(image: &str) -> &str {
    let image = image.split('@').next().unwrap_or(image);
    // a colon after the last slash is a tag (a colon before it is a port)
    match image.rsplit_once(':') {
        Some((repo, tag)) if !tag.contains('/') => repo,
        _ => image,
    }
}

/// The policy scopes that can apply to `image` on the `docker` transport,
/// most specific first, as containers-policy.json(5) looks them up: the
/// reference itself, its repository, each parent namespace, the host, the
/// `*.` wildcards of the host's parent domains, and the transport-wide `""`.
fn scopes(image: &str) -> Vec<String> {
    let mut out = vec![image.to_string()];
    let mut scope = repository(image).to_string();
    loop {
        out.push(scope.clone());
        match scope.rsplit_once('/') {
            Some((parent, _)) => scope = parent.to_string(),
            None => break,
        }
    }
    let mut host = scope.as_str();
    while let Some((_, rest)) = host.split_once('.') {
        out.push(format!("*.{rest}"));
        host = rest;
    }
    out.push(String::new());
    out.dedup();
    out
}

fn types(requirements: &[serde_json::Value]) -> Vec<&str> {
    requirements
        .iter()
        .map(|r| r.get("type").and_then(|t| t.as_str()).unwrap_or("?"))
        .collect()
}

/// The keys of one `sigstoreSigned` requirement that must exist as files.
fn key_files(req: &serde_json::Value) -> Result<Vec<&str>, String> {
    let mut files = Vec::new();
    let mut inline = false;
    for key in ["keyPath", "keyPaths"] {
        match req.get(key) {
            None => {}
            Some(serde_json::Value::String(p)) => files.push(p.as_str()),
            Some(serde_json::Value::Array(a)) => {
                for p in a {
                    files.push(p.as_str().ok_or("a key path is not text")?);
                }
            }
            Some(_) => return Err("a key path is not text".into()),
        }
    }
    for key in ["keyData", "keyDatas"] {
        match req.get(key) {
            None => {}
            Some(serde_json::Value::String(d)) if !d.is_empty() => inline = true,
            Some(serde_json::Value::Array(a)) if !a.is_empty() => inline = true,
            Some(_) => return Err("a key is empty".into()),
        }
    }
    if files.is_empty() && !inline {
        return Err("a signature requirement has no key".into());
    }
    if files.iter().any(|f| !f.starts_with('/')) {
        return Err("a key path is not absolute".into());
    }
    Ok(files)
}

/// `Ok` when pulling `image` is checked against a sigstore key that exists
/// (`key_exists` says so for an absolute path), and the policy's default
/// does not accept anything unchecked. `Err` says in a few words what is
/// missing; it goes to the user.
pub fn check(policy: &str, image: &str, key_exists: &dyn Fn(&str) -> bool) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(policy)
        .map_err(|_| "the container policy of this system can't be read".to_string())?;
    // bootc refuses a default of insecureAcceptAnything; so does this
    let default = v
        .get("default")
        .and_then(|d| d.as_array())
        .ok_or("the container policy has no default")?;
    if types(default).contains(&"insecureAcceptAnything") {
        return Err("the container policy's default accepts any image".into());
    }
    let docker = v.get("transports").and_then(|t| t.get("docker"));
    let requirements = scopes(image)
        .iter()
        .find_map(|s| docker.and_then(|d| d.get(s.as_str())))
        .ok_or("the container policy doesn't ask for a signature on Telamon OS")?
        .as_array()
        .ok_or("the container policy for Telamon OS is malformed")?;
    if requirements.is_empty() {
        return Err("the container policy for Telamon OS has no requirement".into());
    }
    for r in requirements {
        match r.get("type").and_then(|t| t.as_str()) {
            Some("sigstoreSigned") => {
                for file in key_files(r)? {
                    if !key_exists(file) {
                        return Err("the signing key of Telamon OS is missing".into());
                    }
                }
            }
            _ => {
                return Err(
                    "the container policy for Telamon OS asks for something other than its sigstore signature"
                        .into(),
                );
            }
        }
    }
    Ok(())
}

/// Whether a registries.d file tells containers/image to look for sigstore
/// signatures beside `image` (`use-sigstore-attachments: true`; without it a
/// signature the policy requires is never found). Only a setting that applies
/// to the image counts: under `default-docker`, or under a `docker` scope that
/// is the image's repository or one of its namespaces or its registry (the
/// longest such scope decides). The file is read as the small YAML it is
/// (`docker:` / scope / setting, by indentation); wildcards are not followed,
/// so an odd file means "not proven", never a wrong yes.
pub fn attachments_enabled(registries_d_file: &str, image: &str) -> bool {
    let repo = repository(image);
    let applies = |scope: &str| {
        !scope.is_empty() && (repo == scope || repo.starts_with(&format!("{scope}/")))
    };
    let (mut top, mut scope): (&str, String) = ("", String::new());
    let mut default_setting: Option<bool> = None;
    let mut best: Option<(usize, bool)> = None;
    for line in registries_d_file.lines() {
        let body = line.split('#').next().unwrap_or("");
        let indent = body.len() - body.trim_start().len();
        let t = body.trim();
        if t.is_empty() {
            continue;
        }
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().trim_matches(['"', '\'']), value.trim());
        if indent == 0 {
            top = match key {
                "default-docker" => "default-docker",
                "docker" => "docker",
                _ => "",
            };
            scope.clear();
        } else if key == "use-sigstore-attachments" {
            let on = match value {
                "true" => true,
                "false" => false,
                _ => continue,
            };
            match top {
                "default-docker" => default_setting = Some(on),
                "docker"
                    if !scope.is_empty()
                        && applies(&scope)
                        && best.is_none_or(|(len, _)| scope.len() >= len) =>
                {
                    best = Some((scope.len(), on));
                }
                _ => {}
            }
        } else if top == "docker" && value.is_empty() {
            scope = key.to_string();
        }
    }
    best.map(|(_, on)| on).or(default_setting).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: &str = "ghcr.io/eternalcoder454/atlasos:stable";
    const KEY: &str = "/etc/pki/containers/telamon.pub";

    /// What the Telamon OS image build writes into policy.json.
    fn telamon_policy() -> serde_json::Value {
        let signed = serde_json::json!([{
            "type": "sigstoreSigned", "keyPaths": [KEY],
            "signedIdentity": {"type": "matchRepository"}
        }]);
        serde_json::json!({
            "default": [{"type": "reject"}],
            "transports": {
                "docker": {
                    "ghcr.io/eternalcoder454/telamonos": signed,
                    "ghcr.io/eternalcoder454/atlasos": signed,
                    "ghcr.io/eternalcoder454/atlasos-nvidia": signed,
                    "": [{"type": "insecureAcceptAnything"}]
                },
                "containers-storage": {"": [{"type": "insecureAcceptAnything"}]}
            }
        })
    }

    fn have(path: &str) -> bool {
        path == KEY
    }

    fn check_json(v: &serde_json::Value, image: &str) -> Result<(), String> {
        check(&v.to_string(), image, &have)
    }

    #[test]
    fn repositories_lose_tags_and_digests_but_keep_ports() {
        assert_eq!(repository(IMAGE), "ghcr.io/eternalcoder454/atlasos");
        assert_eq!(repository("ghcr.io/a/b"), "ghcr.io/a/b");
        assert_eq!(repository("localhost:5000/a/b"), "localhost:5000/a/b");
        assert_eq!(repository("localhost:5000/a/b:t"), "localhost:5000/a/b");
        assert_eq!(repository("ghcr.io/a/b@sha256:abc"), "ghcr.io/a/b");
        assert_eq!(repository("ghcr.io/a/b:t@sha256:abc"), "ghcr.io/a/b");
    }

    #[test]
    fn the_policy_the_image_build_writes_proves_it() {
        assert_eq!(check_json(&telamon_policy(), IMAGE), Ok(()));
        assert_eq!(
            check_json(
                &telamon_policy(),
                "ghcr.io/eternalcoder454/atlasos-nvidia:stable"
            ),
            Ok(())
        );
    }

    #[test]
    fn a_policy_that_does_not_prove_it_says_why() {
        let p = telamon_policy();
        let why = |v: &serde_json::Value| check_json(v, IMAGE).unwrap_err();

        // a default that accepts anything (bootc itself would refuse)
        let mut v = p.clone();
        v["default"] = serde_json::json!([{"type": "insecureAcceptAnything"}]);
        assert!(why(&v).contains("default accepts any image"));
        let mut v = p.clone();
        v["default"] = serde_json::json!([{"type": "reject"}, {"type": "insecureAcceptAnything"}]);
        assert!(why(&v).contains("default accepts any image"));
        let mut v = p.clone();
        v.as_object_mut().unwrap().remove("default");
        assert!(why(&v).contains("no default"));

        // the image's repository is not covered: the transport-wide entry wins
        let mut v = p.clone();
        v["transports"]["docker"]
            .as_object_mut()
            .unwrap()
            .remove("ghcr.io/eternalcoder454/atlasos");
        assert!(why(&v).contains("asks for something other"), "{}", why(&v));
        // and with no entry at all the default (reject) applies
        v["transports"]["docker"]
            .as_object_mut()
            .unwrap()
            .remove("");
        assert!(why(&v).contains("doesn't ask for a signature"));
        let mut v = p.clone();
        v["transports"].as_object_mut().unwrap().remove("docker");
        assert!(why(&v).contains("doesn't ask for a signature"));

        // the key is missing, or one of several
        let mut v = p.clone();
        v["transports"]["docker"]["ghcr.io/eternalcoder454/atlasos"][0]["keyPaths"] =
            serde_json::json!([KEY, "/etc/pki/containers/old.pub"]);
        assert!(why(&v).contains("key of Telamon OS is missing"));
        assert!(
            check(&p.to_string(), IMAGE, &|_| false)
                .unwrap_err()
                .contains("missing")
        );

        // a requirement of another kind, or none, or without a key
        for bad in [
            serde_json::json!([{"type": "reject"}]),
            serde_json::json!([{"type": "insecureAcceptAnything"}]),
            serde_json::json!([{"type": "signedBy", "keyType": "GPGKeys", "keyPath": KEY}]),
            serde_json::json!([{"type": "sigstoreSigned", "keyPaths": [KEY]}, {"type": "reject"}]),
            serde_json::json!([]),
            serde_json::json!([{"type": "sigstoreSigned"}]),
            serde_json::json!([{"type": "sigstoreSigned", "keyPaths": []}]),
            serde_json::json!([{"type": "sigstoreSigned", "keyPath": 7}]),
            serde_json::json!([{"type": "sigstoreSigned", "keyPath": "relative.pub"}]),
            serde_json::json!([{"type": "sigstoreSigned", "keyData": ""}]),
            serde_json::json!([5]),
            serde_json::json!("sigstoreSigned"),
        ] {
            let mut v = p.clone();
            v["transports"]["docker"]["ghcr.io/eternalcoder454/atlasos"] = bad.clone();
            assert!(check_json(&v, IMAGE).is_err(), "{bad}");
        }

        // not JSON at all, or not an object
        for junk in ["", "{", "[]", "null", "\0", "{\"default\": 5}"] {
            assert!(check(junk, IMAGE, &have).is_err(), "{junk:?}");
        }
    }

    #[test]
    fn a_broader_scope_counts_and_the_most_specific_wins() {
        let signed = serde_json::json!([{"type": "sigstoreSigned", "keyPath": KEY}]);
        let mut v = telamon_policy();
        let d = v["transports"]["docker"].as_object_mut().unwrap();
        d.remove("ghcr.io/eternalcoder454/atlasos");
        d.insert("ghcr.io/eternalcoder454".into(), signed.clone());
        assert_eq!(check_json(&v, IMAGE), Ok(()));
        // inline key data stands in for a key file
        let inline = serde_json::json!([{"type": "sigstoreSigned", "keyData": "LS0tLS1CRUdJTg=="}]);
        v["transports"]["docker"]["ghcr.io/eternalcoder454"] = inline;
        assert_eq!(check(&v.to_string(), IMAGE, &|_| false), Ok(()));
        // the exact reference beats its repository
        v["transports"]["docker"][IMAGE] = serde_json::json!([{"type": "reject"}]);
        assert!(check_json(&v, IMAGE).is_err());
        // a wildcard host is found after the host and its namespaces
        let mut w = telamon_policy();
        let d = w["transports"]["docker"].as_object_mut().unwrap();
        d.remove("ghcr.io/eternalcoder454/atlasos");
        d.insert("*.io".into(), signed);
        assert_eq!(check_json(&w, IMAGE), Ok(()));
    }

    #[test]
    fn signatures_are_looked_for_only_when_registries_d_says_so() {
        let yes = [
            "docker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: true\n",
            "docker:\n  ghcr.io:\n    use-sigstore-attachments: true # yes\n",
            "docker:\n  ghcr.io/eternalcoder454/atlasos:\n    use-sigstore-attachments: true\n",
            "default-docker:\n  use-sigstore-attachments: true\n",
            "docker:\n  \"ghcr.io/eternalcoder454\":\n    lookaside: https://x\n    use-sigstore-attachments: true\n",
            // a more specific scope wins over a broader one, in both directions
            "default-docker:\n  use-sigstore-attachments: false\ndocker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: true\n",
        ];
        for y in yes {
            assert!(attachments_enabled(y, IMAGE), "{y:?}");
        }
        for no in [
            "",
            "use-sigstore-attachments: true\n",
            "docker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: false\n",
            "docker:\n  ghcr.io/eternalcoder454:\n    # use-sigstore-attachments: true\n",
            "docker:\n  ghcr.io/eternalcoder454:\n    xuse-sigstore-attachments: true\n",
            "docker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: truex\n",
            "docker:\n  ghcr.io/eternalcoder454:\n    lookaside: https://example.com\n",
            // another registry, another namespace, a look-alike prefix
            "docker:\n  docker.io:\n    use-sigstore-attachments: true\n",
            "docker:\n  ghcr.io/eternalcoder4540:\n    use-sigstore-attachments: true\n",
            "docker:\n  ghcr.io/eternalcoder454/other:\n    use-sigstore-attachments: true\n",
            "docker:\n  ghcr.io/eternalcoder454/atlasos:\n    use-sigstore-attachments: false\n  ghcr.io:\n    use-sigstore-attachments: true\n",
            "default-docker:\n  use-sigstore-attachments: true\ndocker:\n  ghcr.io/eternalcoder454:\n    use-sigstore-attachments: false\n",
        ] {
            assert!(!attachments_enabled(no, IMAGE), "{no:?}");
        }
    }
}
