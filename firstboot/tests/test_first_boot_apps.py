"""Tests for telamon-first-boot-apps, with fake flatpak/mise/toolbox/curl/gdbus on PATH.

The script is imported and its module globals are patched (paths, checksum, root
checks); the production script has no environment overrides.
"""
import hashlib
import importlib.machinery
import importlib.util
import json
import os
import stat
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parent / "telamon-first-boot-apps"
CATALOG = HERE.parent / "apps.json"

_loader = importlib.machinery.SourceFileLoader("fba", str(SCRIPT))
fba = importlib.util.module_from_spec(importlib.util.spec_from_loader("fba", _loader))
_loader.exec_module(fba)

MISE_BODY = '#!/bin/sh\necho "mise $*" >> "$FAKE_LOG"\n'

# Every fake logs its arguments to $FAKE_LOG. FAIL_<NAME>=1 makes it exit 1, and
# FAIL_MATCH=<text> makes any fake fail when its arguments contain the text.
FAKE = """#!/bin/sh
echo "$(basename "$0") $*" >> "$FAKE_LOG"
name=$(basename "$0" | tr a-z A-Z)
eval "fail=\\${FAIL_$name:-}"
[ -n "$fail" ] && exit 1
if [ -n "${FAIL_MATCH:-}" ]; then case "$*" in *"$FAIL_MATCH"*) exit 1;; esac; fi
case "$(basename "$0")" in
  curl) while [ $# -gt 0 ]; do [ "$1" = -o ] && printf %s "$FAKE_MISE" > "$2"; shift; done ;;
  toolbox) if [ "$1" = list ]; then
      echo "CONTAINER ID  CONTAINER NAME  CREATED"
      [ -n "${TB_EXISTS:-}" ] && echo "abc  $TB_EXISTS  now"
    fi ;;
esac
exit 0
"""


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        t = Path(self.tmp.name)
        self.bin, self.state, self.home, self.run_dir = t / "bin", t / "state", t / "home", t / "run"
        for d in (self.bin, self.state, self.home, self.run_dir):
            d.mkdir()
        for n in ("flatpak", "mise", "toolbox", "curl", "gdbus"):
            p = self.bin / n
            p.write_text(FAKE)
            p.chmod(p.stat().st_mode | stat.S_IXUSR)
        self.record, self.status, self.log = t / "rec.json", t / "rec.status", t / "log"
        # What it was called before the rename: nothing there unless a test puts it.
        self.old_record, self.old_status, self.old_state = t / "old" / "rec.json", t / "old" / "rec.status", t / "old" / "state"
        self.old_record.parent.mkdir()
        self.log.touch()
        (t / "os-release").write_text('NAME=Fedora\nVERSION_ID=44\n')
        env = {"PATH": "%s:/usr/bin:/bin" % self.bin, "FAKE_LOG": str(self.log),
               "FAKE_MISE": MISE_BODY}
        p = mock.patch.dict(os.environ, env)
        p.start()
        self.addCleanup(p.stop)
        for k, v in dict(RECORD=self.record, STATUS=self.status, CATALOG=CATALOG,
                         LEGACY_RECORD=self.old_record, LEGACY_STATUS=self.old_status,
                         STATE_DIR=self.state, LEGACY_STATE_DIR=self.old_state, BIN_DIR=self.home / "bin", OS_RELEASE=t / "os-release", PCI_DEVICES=t / "no-pci",
                         RUNTIME_DIR=self.run_dir, REQUIRE_ROOT=False, CHECK_OWNER=False,
                         MISE_SHA256=hashlib.sha256(MISE_BODY.encode()).hexdigest()).items():
            q = mock.patch.object(fba, k, v)
            q.start()
            self.addCleanup(q.stop)
        q = mock.patch.object(fba.time, "sleep")
        q.start()
        self.addCleanup(q.stop)

    def write_record(self, apps):
        self.record.write_text(json.dumps({"version": 1, "apps": apps}))

    def run_mode(self, mode, **env):
        with mock.patch.dict(os.environ, {k: str(v) for k, v in env.items()}):
            return fba.main(["x", mode])

    def calls(self, prefix=""):
        return [c for c in self.log.read_text().splitlines() if c.startswith(prefix)]

    def st(self):
        return json.loads(self.status.read_text())

    def rec_apps(self):
        return json.loads(self.record.read_text())["apps"]


class SystemMode(Base):
    def test_success_keeps_only_per_account_entries(self):
        self.write_record(["firefox", "gh", "debug"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertEqual(self.rec_apps(), ["gh", "debug"])
        self.assertTrue(self.calls("flatpak remote-add --if-not-exists --system flathub https://dl.flathub.org/"))
        self.assertTrue(self.calls("flatpak install --system -y --noninteractive flathub org.mozilla.firefox"))
        self.assertEqual(self.st()["state"], "done")

    def pci(self, *devices):
        """A fake /sys/bus/pci/devices with (vendor, class) pairs."""
        root = Path(self.tmp.name) / "pci"
        for n, (vendor, cls) in enumerate(devices):
            d = root / ("0000:00:%02x.0" % n)
            d.mkdir(parents=True)
            (d / "vendor").write_text(vendor + "\n")
            (d / "class").write_text(cls + "\n")
        root.mkdir(exist_ok=True)
        q = mock.patch.object(fba, "PCI_DEVICES", root)
        q.start()
        self.addCleanup(q.stop)

    def alpaca_refs(self):
        return [c.rsplit(" ", 1)[1] for c in self.calls("flatpak install")]

    def test_alpaca_comes_with_its_own_ollama_engine(self):
        self.pci(("0x10de", "0x030000"))  # NVIDIA: no AMD add-on
        self.write_record(["ollama", "alpaca"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertEqual(self.alpaca_refs(), ["com.jeffser.Alpaca", "com.jeffser.Alpaca.Plugins.Ollama"])
        self.assertEqual(self.rec_apps(), ["ollama"])  # the mise tool waits for the account
        self.assertEqual(self.calls("mise"), [])

    def test_alpaca_adds_the_amd_plugin_on_an_amd_gpu(self):
        self.pci(("0x8086", "0x060000"), ("0x1002", "0x030000"))
        self.write_record(["alpaca"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertEqual(self.alpaca_refs(), ["com.jeffser.Alpaca", "com.jeffser.Alpaca.Plugins.Ollama",
                                              "com.jeffser.Alpaca.Plugins.AMD"])
        self.assertFalse(self.record.exists())

    def test_an_amd_device_that_is_not_a_display_does_not_count(self):
        self.pci(("0x1002", "0x040300"))  # AMD audio, no GPU
        self.write_record(["alpaca"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertNotIn("com.jeffser.Alpaca.Plugins.AMD", self.alpaca_refs())

    def test_a_missing_sysfs_means_no_amd_plugin(self):
        # (setUp already points PCI_DEVICES at a directory that does not exist)
        self.write_record(["alpaca"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertEqual(self.alpaca_refs(), ["com.jeffser.Alpaca", "com.jeffser.Alpaca.Plugins.Ollama"])

    def test_a_failed_addon_keeps_the_app_for_a_retry(self):
        self.pci(("0x1002", "0x030000"))
        self.write_record(["alpaca"])
        self.assertEqual(self.run_mode("system", FAIL_MATCH="Plugins.AMD"), 1)
        self.assertEqual(self.rec_apps(), ["alpaca"])
        self.assertEqual(self.st()["failed"], ["alpaca"])

    def test_success_deletes_record_when_nothing_left(self):
        self.write_record(["firefox", "brave"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertFalse(self.record.exists())
        self.assertEqual(len(self.calls("flatpak install")), 2)  # one install per app

    def test_one_failed_app_keeps_only_that_id(self):
        self.write_record(["firefox", "brave", "gh"])
        self.assertEqual(self.run_mode("system", FAIL_MATCH="com.brave.Browser"), 1)
        self.assertEqual(self.rec_apps(), ["brave", "gh"])
        st = self.st()
        self.assertEqual((st["state"], st["failed"]), ("failed", ["brave"]))
        self.assertTrue(st["message"])

    def test_remote_failure_keeps_everything(self):
        self.write_record(["firefox", "gh"])
        self.assertEqual(self.run_mode("system", FAIL_FLATPAK=1), 1)
        self.assertEqual(self.rec_apps(), ["firefox", "gh"])

    def test_unknown_id_is_reported_and_kept_not_run(self):
        self.write_record(["firefox", "evil; rm -rf /", "futureapp"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertNotIn("evil", self.log.read_text())
        self.assertNotIn("futureapp", self.log.read_text())
        self.assertEqual(self.rec_apps(), ["futureapp"])
        st = self.st()
        self.assertEqual((st["state"], st["unknown"]), ("partial", ["futureapp"]))
        self.assertIn("aren't available", st["message"])

    def test_partial_status_is_stable_across_runs(self):
        self.write_record(["futureapp"])
        self.run_mode("system")
        first = self.st()["updated"]
        with mock.patch.object(fba.time, "time", return_value=first + 500):
            self.run_mode("system")
        self.assertEqual(self.st()["updated"], first)

    def test_corrupt_record_set_aside_without_retry(self):
        self.record.write_text("not json")
        self.assertEqual(self.run_mode("system"), fba.EX_PERMANENT)
        self.assertFalse(self.record.exists())
        self.assertTrue(self.record.with_name("rec.json.bad").exists())
        self.assertTrue(self.st()["permanent"])
        self.assertEqual(self.calls(), [])

    def test_wrong_version_and_wrong_type(self):
        for text in ('{"version": 2, "apps": []}', '[1, 2]'):
            self.record.write_text(text)
            self.assertEqual(self.run_mode("system"), fba.EX_PERMANENT, text)

    @unittest.skipIf(os.geteuid() == 0, "needs a non-root owner")
    def test_bad_owner_refused(self):
        self.write_record(["firefox"])
        with mock.patch.object(fba, "CHECK_OWNER", True):
            self.assertEqual(self.run_mode("system"), fba.EX_PERMANENT)
        self.assertEqual(self.calls(), [])

    def test_missing_flatpak_is_permanent(self):
        self.write_record(["firefox"])
        with mock.patch.object(fba.subprocess, "run", side_effect=FileNotFoundError("flatpak")):
            self.assertEqual(self.run_mode("system"), fba.EX_PERMANENT)
        self.assertEqual(self.rec_apps(), ["firefox"])

    def test_stale_running_status_is_ignored(self):
        self.status.write_text(json.dumps({"state": "running", "updated": 1}))
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls("gdbus"), [])


class UserMode(Base):
    def test_installs_mise_tools_and_toolbox_then_remembers(self):
        self.write_record(["gh", "debug"])
        self.assertEqual(self.run_mode("user"), 0)
        mise = self.home / "bin" / "mise"
        self.assertTrue(mise.exists() and os.access(mise, os.X_OK))
        self.assertTrue(self.calls("mise use -g gh"))
        self.assertTrue(self.calls("toolbox create -y -c fedora-toolbox-44"))
        self.assertTrue(self.calls("toolbox run -c fedora-toolbox-44 sudo dnf install -y gdb strace perf"))
        self.assertEqual(set((self.state / "first-boot-apps.done").read_text().split()),
                         {"gh", "debug"})
        self.assertEqual(len([c for c in self.calls("gdbus") if "Adding your apps" in c]), 1)

    def test_ollama_comes_through_mise_with_no_models(self):
        self.write_record(["ollama"])
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls("mise"), ["mise use -g ollama"])  # not "ollama pull" or "run"
        self.assertEqual(self.calls("toolbox"), [])
        self.assertEqual((self.state / "first-boot-apps.done").read_text().split(), ["ollama"])

    def test_done_file_prevents_repeats(self):
        self.write_record(["gh", "debug"])
        self.assertEqual(self.run_mode("user"), 0)
        self.log.write_text("")
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls(), [])

    def test_checksum_mismatch_refuses_mise_and_does_not_loop(self):
        self.write_record(["gh"])
        self.assertEqual(self.run_mode("user", FAKE_MISE="tampered"), 0)
        self.assertEqual(list((self.home / "bin").iterdir()), [])
        self.assertIn("did not match its checksum", " ".join(self.calls("gdbus")))
        self.log.write_text("")
        self.assertEqual(self.run_mode("user", FAKE_MISE="tampered"), 0)
        self.assertEqual(self.calls(), [])

    def test_non_x86_64_is_permanent(self):
        self.write_record(["gh"])
        with mock.patch.object(fba.platform, "machine", return_value="aarch64"):
            self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls("curl"), [])
        self.assertIn("64-bit", " ".join(self.calls("gdbus")))

    def test_offline_download_retries_then_fails(self):
        self.write_record(["gh"])
        self.assertEqual(self.run_mode("user", FAIL_CURL=1), 1)
        self.assertEqual(len(self.calls("curl")), 10)
        self.assertFalse((self.state / "first-boot-apps.done").exists())

    def test_start_notice_once_per_boot(self):
        self.write_record(["gh"])
        self.run_mode("user", FAIL_CURL=1)
        self.run_mode("user", FAIL_CURL=1)
        self.assertEqual(len([c for c in self.calls("gdbus") if "Adding your apps" in c]), 1)

    def test_only_the_default_toolbox_counts(self):
        self.write_record(["debug"])
        self.assertEqual(self.run_mode("user", TB_EXISTS="other-box"), 0)
        self.assertTrue(self.calls("toolbox create"))

    def test_existing_default_toolbox_is_reused(self):
        self.write_record(["debug"])
        self.assertEqual(self.run_mode("user", TB_EXISTS="fedora-toolbox-44"), 0)
        self.assertFalse(self.calls("toolbox create"))

    def test_reports_flatpak_result_once(self):
        self.status.write_text(json.dumps({"state": "done", "message": "", "apps": ["firefox"], "updated": 1}))
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(len(self.calls("gdbus")), 1)
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(len(self.calls("gdbus")), 1)

    def test_flatpak_failure_notification_has_reason(self):
        self.status.write_text(json.dumps({"state": "failed", "message": "Adding Flathub did not finish.",
                                           "apps": ["firefox"], "updated": 2}))
        self.run_mode("user")
        self.assertIn("Adding Flathub did not finish.", self.calls("gdbus")[0])

    def test_partial_notification(self):
        self.status.write_text(json.dumps({"state": "partial", "updated": 3,
                                           "message": "Some apps aren't available in this version."}))
        self.run_mode("user")
        self.assertIn("available in this version", self.calls("gdbus")[0])

    def test_status_must_be_a_dict(self):
        self.status.write_text("[1]")
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls("gdbus"), [])


class UnderTheOldNames(Base):
    """A machine whose first start began (or ended) as atlas-first-boot-apps."""

    def write_old_record(self, apps):
        self.old_record.write_text(json.dumps({"version": 1, "apps": apps}))

    def test_a_finished_first_start_does_not_run_again(self):
        # nothing is pending under either name: no record, no calls, in both modes
        self.assertEqual(self.run_mode("system"), 0)
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls(), [])
        self.assertFalse(self.record.exists() or self.old_record.exists())

    def test_the_old_record_is_read_and_moves_to_the_new_name(self):
        self.write_old_record(["firefox", "gh"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertTrue(self.calls("flatpak install --system -y --noninteractive flathub org.mozilla.firefox"))
        self.assertEqual(self.rec_apps(), ["gh"])
        self.assertFalse(self.old_record.exists(), "the work left is only in one place")
        self.assertEqual(self.st()["state"], "done")

    def test_an_old_record_with_only_account_entries_moves_without_installing(self):
        self.write_old_record(["gh", "debug"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertEqual(self.calls("flatpak"), [])
        self.assertEqual(self.rec_apps(), ["gh", "debug"])
        self.assertFalse(self.old_record.exists())

    def test_the_new_record_wins_and_both_go_when_it_is_done(self):
        self.write_record(["firefox"])
        self.write_old_record(["brave", "firefox"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertEqual(len(self.calls("flatpak install")), 1)
        self.assertFalse(self.record.exists() or self.old_record.exists())

    def test_a_damaged_old_record_is_set_aside_without_retry(self):
        self.old_record.write_text("not json")
        self.assertEqual(self.run_mode("system"), fba.EX_PERMANENT)
        self.assertFalse(self.old_record.exists())
        self.assertTrue(self.old_record.with_name("rec.json.bad").exists())

    def test_done_markers_under_the_old_name_count(self):
        self.write_old_record(["gh", "debug"])
        self.old_state.mkdir()
        (self.old_state / "first-boot-apps.done").write_text("debug\ngh\n")
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls(), [], "nothing is installed again")
        self.assertFalse((self.state / "first-boot-apps.done").exists())

    def test_a_new_done_marker_keeps_what_the_old_one_said(self):
        self.write_old_record(["gh", "debug"])
        self.old_state.mkdir()
        (self.old_state / "first-boot-apps.done").write_text("gh\n")
        self.assertEqual(self.run_mode("user"), 0)
        self.assertTrue(self.calls("toolbox create"))
        self.assertFalse(self.calls("mise use"), "gh was done under the old name")
        self.assertEqual(set((self.state / "first-boot-apps.done").read_text().split()), {"gh", "debug"})

    def test_a_result_shown_under_the_old_name_is_not_shown_again(self):
        self.old_state.mkdir()
        stamp = "done:1"
        (self.old_state / "first-boot-apps.notified").write_text(stamp + "\n")
        self.old_status.write_text(json.dumps({"state": "done", "message": "", "apps": ["firefox"], "updated": 1}))
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(self.calls("gdbus"), [])

    def test_a_result_waiting_under_the_old_name_is_shown_once(self):
        self.old_status.write_text(json.dumps({"state": "done", "message": "", "apps": ["firefox"], "updated": 1}))
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(len(self.calls("gdbus")), 1)
        self.assertEqual(self.run_mode("user"), 0)
        self.assertEqual(len(self.calls("gdbus")), 1)

    def test_a_new_status_replaces_the_old_one(self):
        self.old_status.write_text(json.dumps({"state": "done", "message": "", "apps": ["x"], "updated": 1}))
        self.write_record(["firefox"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertTrue(self.status.exists())
        self.assertFalse(self.old_status.exists())


class Packaging(unittest.TestCase):
    """What firstboot/install.sh puts in an image: the old names still lead to the new files."""

    def test_the_old_unit_and_script_names_are_links_to_the_new_ones(self):
        import subprocess
        with tempfile.TemporaryDirectory() as d:
            subprocess.run([str(HERE.parent / "install.sh"), d], check=True)
            root = Path(d)
            lib = root / "usr/lib/systemd"
            for kind in ("system", "user"):
                new, old = lib / kind / "telamon-first-boot-apps.service", lib / kind / "atlas-first-boot-apps.service"
                self.assertTrue(new.is_file() and not new.is_symlink())
                self.assertEqual(os.readlink(old), "telamon-first-boot-apps.service")
                self.assertTrue(old.exists())
            for target in ("multi-user.target", "graphical-session.target"):
                kind = "system" if target == "multi-user.target" else "user"
                for name in ("telamon", "atlas"):
                    self.assertTrue((lib / kind / (target + ".wants") / (name + "-first-boot-apps.service")).exists(), name)
            script = root / "usr/libexec/atlasos/atlas-first-boot-apps"
            self.assertEqual(script.resolve(), (root / "usr/libexec/telamon/telamon-first-boot-apps").resolve())
            self.assertTrue(os.access(script, os.X_OK))
            catalog = root / "usr/share/atlasos/first-boot-apps.json"
            self.assertEqual(catalog.read_bytes(), (root / "usr/share/telamon/first-boot-apps.json").read_bytes())
            # one profile.d script activates mise; the old name is an empty stand-in
            self.assertNotIn("activate", (root / "etc/profile.d/atlas-mise.sh").read_text())
            self.assertIn("activate", (root / "etc/profile.d/telamon-mise.sh").read_text())

    def test_both_names_of_the_record_start_the_units(self):
        for unit in ("telamon-first-boot-apps.service", "telamon-first-boot-apps.user.service"):
            text = (HERE.parent / "units" / unit).read_text()
            for path in ("/var/lib/telamon/first-boot-apps.json", "/var/lib/atlasos/first-boot-apps.json"):
                self.assertIn("ConditionPathExists=|" + path + "\n", text, unit)
        user = (HERE.parent / "units" / "telamon-first-boot-apps.user.service").read_text()
        for path in ("/var/lib/telamon/first-boot-apps.status", "/var/lib/atlasos/first-boot-apps.status"):
            self.assertIn("ConditionPathExists=|" + path + "\n", user)


class Notify(Base):
    def test_quote_and_backslash_are_escaped(self):
        fba.notify("It's done", "a\\b\nc")
        line = self.calls("gdbus")[0]
        self.assertIn("'It\\'s done'", line)
        self.assertIn("'a\\\\b c'", line)
        self.assertEqual(fba.gv("it's"), "'it\\'s'")


if __name__ == "__main__":
    unittest.main()
