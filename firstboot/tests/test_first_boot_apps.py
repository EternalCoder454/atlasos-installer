"""Tests for atlas-first-boot-apps, with fake flatpak/mise/toolbox/curl/gdbus on PATH.

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
SCRIPT = HERE.parent / "atlas-first-boot-apps"
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
        self.log.touch()
        (t / "os-release").write_text('NAME=Fedora\nVERSION_ID=44\n')
        env = {"PATH": "%s:/usr/bin:/bin" % self.bin, "FAKE_LOG": str(self.log),
               "FAKE_MISE": MISE_BODY}
        p = mock.patch.dict(os.environ, env)
        p.start()
        self.addCleanup(p.stop)
        for k, v in dict(RECORD=self.record, STATUS=self.status, CATALOG=CATALOG,
                         STATE_DIR=self.state, BIN_DIR=self.home / "bin", OS_RELEASE=t / "os-release",
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

    def test_local_ai_chat_app_is_a_system_flatpak_and_ollama_waits_for_the_account(self):
        self.write_record(["ollama", "alpaca"])
        self.assertEqual(self.run_mode("system"), 0)
        self.assertTrue(self.calls("flatpak install --system -y --noninteractive flathub com.jeffser.Alpaca"))
        self.assertEqual(self.rec_apps(), ["ollama"])
        self.assertEqual(self.calls("mise"), [])  # no model or tool is fetched by the system part

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


class Notify(Base):
    def test_quote_and_backslash_are_escaped(self):
        fba.notify("It's done", "a\\b\nc")
        line = self.calls("gdbus")[0]
        self.assertIn("'It\\'s done'", line)
        self.assertIn("'a\\\\b c'", line)
        self.assertEqual(fba.gv("it's"), "'it\\'s'")


if __name__ == "__main__":
    unittest.main()
