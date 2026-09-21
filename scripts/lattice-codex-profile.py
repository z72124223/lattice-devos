#!/usr/bin/env python3
"""Export, preview and explicitly apply portable Codex preferences (Python 3.11+).

No auth, MCP, providers, permissions, safety policies, paths or instructions are
portable here. The existing --config FILE --output FILE export CLI is retained.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import uuid

SPEC = importlib.util.spec_from_file_location("lattice_profile_config", Path(__file__).with_name("lattice-mcp-config.py"))
CONFIG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONFIG)
Rejected = CONFIG.Rejected
SCHEMA = "lattice.codex-portable-profile.v1"
# Official keys: https://developers.openai.com/codex/config-reference
# Effort availability remains model/client dependent, including desktop efforts.
EFFORTS = {"none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"}
VALUES = {
    "model_reasoning_effort": EFFORTS,
    "plan_mode_reasoning_effort": EFFORTS,
    "model_reasoning_summary": {"auto", "concise", "detailed", "none"},
    "model_verbosity": {"low", "medium", "high"},
    "personality": {"none", "friendly", "pragmatic"},
}
KEEP_TOP = {"model", *VALUES}
MODEL = re.compile(r"(?:gpt-[0-9]+(?:\.[0-9]+)?|o[1-9][0-9]*)(?:-[a-z0-9]+)*\Z")
SECRET_VALUE = re.compile(r"(?:sk-|gh[pousr]_|github_pat_|eyJ|bearer|password|token|secret|credential|api[_-]?key)", re.I)


def safe_value(key: str, value):
    """Only closed preference values are exportable; never recursively copy data."""
    if key not in KEEP_TOP or not isinstance(value, str) or len(value) > 80:
        return None
    if SECRET_VALUE.search(value) or any(c in value for c in "/\\:\r\n\0"):
        return None
    if key == "model":
        return value if MODEL.fullmatch(value) else None
    return value if value in VALUES[key] else None


def known_provider(data: dict) -> bool:
    return (data.get("model_provider", "openai") == "openai"
            and not any(key in data for key in ("model_providers", "openai_base_url", "oss_provider")))


def same_file(left: Path, right: Path) -> bool:
    if os.path.normcase(os.path.abspath(left)) == os.path.normcase(os.path.abspath(right)):
        return True
    return left.exists() and right.exists() and os.path.samefile(left, right)


def export(config: Path, output: Path) -> dict:
    if same_file(config, output):
        raise Rejected("PROFILE_OUTPUT_OVERLAPS_CONFIG")
    data = CONFIG.parse(CONFIG.read_config(config))
    portable = {key: value for key in sorted(KEEP_TOP)
                if (value := safe_value(key, data.get(key))) is not None
                and (key == "personality" or known_provider(data))}
    result = {"schema": SCHEMA, "source": "local Codex preferences", "preferences": portable,
              "secrets_included": False, "absolute_paths_included": False,
              "apply_policy": "review_exact_plan; explicitly_confirm_conflicts; preserve_other_settings",
              "model_availability": "NOT_VERIFIED"}
    write_report(output, result)
    return result


def read_profile(profile: Path) -> dict:
    CONFIG.regular_path(profile)
    if not profile.is_file() or profile.stat().st_size > 65536:
        raise Rejected("PROFILE_FILE_REJECTED")
    try:
        data = json.loads(profile.read_text(encoding="utf-8-sig"))
    except (ValueError, UnicodeError):
        raise Rejected("PROFILE_JSON_INVALID") from None
    if not isinstance(data, dict) or data.get("schema") != SCHEMA or not isinstance(data.get("preferences"), dict):
        raise Rejected("PROFILE_SCHEMA_REJECTED")
    preferences = data["preferences"]
    if any(safe_value(key, value) != value or value is None for key, value in preferences.items()):
        raise Rejected("PROFILE_PREFERENCE_REJECTED")
    return preferences


def make_plan(before: bytes, preferences: dict, *, existed: bool) -> dict:
    current = CONFIG.parse(before)
    changes = []
    for key, value in sorted(preferences.items()):
        if current.get(key) == value:
            continue
        present = key in current
        visible = (safe_value(key, current[key]) or "<redacted nonportable value>") if present else None
        changes.append({"key": key, "before": visible, "after": value, "conflict": present})
    blocked = bool(changes and not known_provider(current) and any(x["key"] != "personality" for x in changes))
    result = {"schema": "lattice.codex-profile-plan.v1", "config_sha256": CONFIG.digest(before),
              "config_exists": existed, "preferences_sha256": CONFIG.digest(CONFIG.json_bytes(preferences)),
              "changes": changes, "status": "PROVIDER_REVIEW_REQUIRED" if blocked else
              "REVIEW_REQUIRED" if any(x["conflict"] for x in changes) else "READY" if changes else "UNCHANGED",
              "model_availability": "NOT_VERIFIED"}
    result["plan_sha256"] = CONFIG.digest(CONFIG.json_bytes(result))
    return result


def plan(config: Path, profile: Path) -> dict:
    preferences = read_profile(profile)
    before = CONFIG.read_config(config)
    return make_plan(before, preferences, existed=config.exists())


def rewrite_preferences(before: bytes, preferences: dict) -> bytes:
    """Edit only simple root scalar assignments; reject ambiguous TOML layouts."""
    current = CONFIG.parse(before)
    bom = b"\xef\xbb\xbf" if before.startswith(b"\xef\xbb\xbf") else b""
    text = before[len(bom):].decode("utf-8")
    lines = text.splitlines(keepends=True)
    newline = "\r\n" if "\r\n" in text else "\n"
    additions = []
    for key, value in sorted(preferences.items()):
        if current.get(key) == value:
            continue
        encoded = json.dumps(value, ensure_ascii=False)
        if key not in current:
            additions.append(f"{key} = {encoded}{newline}")
            continue
        pattern = re.compile(rf'''^(?P<prefix>[ \t]*(?:{key}|"{key}"|'{key}')[ \t]*=[ \t]*)(?:"(?:[^"\\\r\n]|\\.)*"|'[^'\r\n]*'|true|false|[0-9]+)(?P<suffix>[ \t]*(?:\#[^\r\n]*)?(?:\r?\n)?)$''')
        candidates = []
        for index, line in enumerate(lines):
            if line.lstrip().startswith("["):
                break
            if match := pattern.fullmatch(line):
                candidates.append((index, match))
        if len(candidates) != 1:
            raise Rejected("PROFILE_TOML_LAYOUT_REQUIRES_MANUAL_REVIEW")
        index, match = candidates[0]
        lines[index] = match["prefix"] + encoded + match["suffix"]
    after = bom + ("".join(additions) + "".join(lines)).encode("utf-8")
    expected = {**current, **preferences}
    if CONFIG.parse(after) != expected:
        raise Rejected("PROFILE_UNRELATED_SETTINGS_CHANGE_REJECTED")
    return after


def apply(config: Path, profile: Path, plan_sha256: str, *, confirm_conflicts: bool = False) -> dict:
    preferences = read_profile(profile)
    folder, _, pending = CONFIG.locations(config)
    if not config.parent.is_dir():
        raise Rejected("CONFIG_PARENT_MISSING")
    with CONFIG.manager_lock(folder):
        if pending.exists():
            raise Rejected("CONFIG_RECOVERY_REQUIRED")
        before = CONFIG.read_config(config)
        existed = config.exists()
        preview = make_plan(before, preferences, existed=existed)
        if preview["plan_sha256"] != plan_sha256:
            raise Rejected("PROFILE_PLAN_STALE_OR_MISSING")
        if preview["status"] == "PROVIDER_REVIEW_REQUIRED":
            raise Rejected("PROFILE_PROVIDER_REVIEW_REQUIRED")
        if preview["status"] == "REVIEW_REQUIRED" and not confirm_conflicts:
            raise Rejected("PROFILE_CONFLICT_CONFIRMATION_REQUIRED")
        if not preview["changes"]:
            return {"status": "UNCHANGED", "backup_saved": False}
        after = rewrite_preferences(before, preferences)
        if existed and not config.stat().st_mode & 0o222:
            raise Rejected("CONFIG_READ_ONLY")
        backup = folder / (uuid.uuid4().hex + ".profile.config.bak")
        descriptor = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            if existed:
                CONFIG.copy_access(config, backup)
            stream.write(before)
            stream.flush()
            os.fsync(stream.fileno())
        if config.exists() != existed or CONFIG.read_config(config) != before:
            raise Rejected("CONFIG_CHANGED_DURING_OPERATION")
        if os.name == "nt":
            CONFIG.write_config(config, before, after, existed)
        else:
            CONFIG.atomic_write(config, after)
        if CONFIG.read_config(config) != after:
            raise Rejected("PROFILE_CONFIG_READBACK_MISMATCH")
        return {"status": "PREFERENCES_APPLIED", "backup_saved": True, "backup": str(backup),
                "changed_keys": [x["key"] for x in preview["changes"]],
                "other_settings": "PRESERVED", "model_availability": "NOT_VERIFIED"}


def write_report(output: Path, value: dict) -> None:
    CONFIG.regular_path(output)
    output.parent.mkdir(parents=True, exist_ok=True)
    CONFIG.atomic_write(output, CONFIG.json_bytes(value))


def confirm_preferences(preview: dict) -> bool:
    """Ask about the exact redacted plan; default to keeping the user's choices."""
    if os.name != "nt":
        raise Rejected("PROFILE_REVIEW_DIALOG_UNAVAILABLE")
    import ctypes
    from ctypes import wintypes
    message = "LATTICE 的建議偏好與您目前的 Codex 設定不同：\n\n"
    for change in preview["changes"]:
        before = change["before"]
        if before is None:
            before = "未設定"
        elif before == "<redacted nonportable value>":
            before = "非可攜設定（已隱藏）"
        message += f'{change["key"]}\n  原本：{before}\n  建議：{change["after"]}\n\n'
    message += "同意套用以上偏好嗎？\n選「是」會先備份原設定；選「否」會保留您原本的設定。\n帳號、MCP 與安全政策不會變更。"
    api = ctypes.WinDLL("user32", use_last_error=True)
    api.MessageBoxW.argtypes = [wintypes.HWND, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.UINT]
    api.MessageBoxW.restype = ctypes.c_int
    choice = api.MessageBoxW(None, message, "LATTICE：Codex 偏好差異", 0x4 | 0x20 | 0x100)
    if choice not in (6, 7):
        raise Rejected("PROFILE_REVIEW_DIALOG_UNAVAILABLE")
    return choice == 6


def setup(config: Path, profile: Path, *, interactive: bool = False) -> dict:
    """Installer entry: fill missing defaults and preserve unapproved conflicts."""
    preview = plan(config, profile)
    status = preview["status"]
    if status == "UNCHANGED":
        return {"status": "UNCHANGED", "backup_saved": False}
    if status == "PROVIDER_REVIEW_REQUIRED":
        return {**preview, "preferences_preserved": True}
    if status == "REVIEW_REQUIRED":
        if not interactive:
            return {**preview, "preferences_preserved": True}
        if not confirm_preferences(preview):
            return {**preview, "status": "USER_PREFERENCES_PRESERVED", "preferences_preserved": True}
    # apply re-reads both inputs under the manager lock and rejects any change
    # since the preview/dialog, instead of treating a stale Yes as new consent.
    return apply(config, profile, preview["plan_sha256"], confirm_conflicts=status == "REVIEW_REQUIRED")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", nargs="?", default="export", choices=["export", "plan", "apply", "setup"])
    parser.add_argument("--config", type=Path, default=Path(os.environ.get("CODEX_HOME", Path.home()/".codex"))/"config.toml")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--plan-sha256", default="")
    parser.add_argument("--confirm-conflicts", action="store_true")
    parser.add_argument("--interactive", action="store_true")
    args = parser.parse_args()
    config = args.config.absolute()
    if args.action == "export" and args.output is None:
        parser.error("export requires --output")
    if args.action != "export" and args.profile is None:
        parser.error("plan/apply/setup require --profile")
    try:
        if args.output and (same_file(args.output, config) or (args.profile and same_file(args.output, args.profile))):
            raise Rejected("PROFILE_OUTPUT_OVERLAPS_INPUT")
        if args.action == "export":
            result = export(config, args.output)
        else:
            if args.action == "setup":
                result = setup(config, args.profile, interactive=args.interactive)
            else:
                result = plan(config, args.profile) if args.action == "plan" else apply(config, args.profile, args.plan_sha256, confirm_conflicts=args.confirm_conflicts)
            if args.output:
                write_report(args.output, result)
        print(json.dumps(result, ensure_ascii=True))
        return 2 if args.action == "setup" and result["status"] == "PROVIDER_REVIEW_REQUIRED" else 0
    except (Rejected, OSError) as error:
        print(json.dumps({"status": "BLOCKED", "code": str(error) if isinstance(error, Rejected) else "PROFILE_IO_REJECTED"}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
