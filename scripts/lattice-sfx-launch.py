"""Wait for the actual setup and retain its result beyond SFX temp cleanup."""
import ctypes
import json
import os
from pathlib import Path
import subprocess
import sys
import time


def save_result(destination: Path, result: dict) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name("sfx-result-" + result["run_id"] + ".tmp")
    with temporary.open("x", encoding="utf-8") as stream:
        json.dump(result, stream)
    temporary.replace(destination)


def main() -> int:
    destination = Path(os.environ["LOCALAPPDATA"]) / "LATTICE/sfx-last-result.json"
    result = {"schema": "lattice.sfx-result.v1", "status": "SETUP_STARTING", "run_id": str(time.time_ns()),
              "setup_exit_code": None, "reboot_requested": False,
              "started_at": int(time.time()), "finished_at": None}
    save_result(destination, result)  # Replace stale success before attempting setup.
    failure = "WINDOWS_SYSTEM_DIRECTORY_UNAVAILABLE"
    try:
        root = Path(__file__).resolve().parent
        buffer = ctypes.create_unicode_buffer(32768)
        length = ctypes.windll.kernel32.GetSystemDirectoryW(buffer, len(buffer))
        if not 0 < length < len(buffer):
            raise OSError()
        # cmd.exe consumes its own quoting rules, not C runtime list2cmdline escaping.
        command = ('"' + str(Path(buffer.value) / "cmd.exe") + '" /d /s /c ""'
                   + str(root / "Install-LATTICE.cmd") + '""')
        failure = "SETUP_COULD_NOT_START"
        code = subprocess.run(command, cwd=root).returncode
        result.update(status="SETUP_EXITED", setup_exit_code=code, reboot_requested=code == 3)
    except OSError:
        result.update(status="SETUP_FAILED", setup_exit_code=2, code=failure)
    result["finished_at"] = int(time.time())
    save_result(destination, result)
    return result["setup_exit_code"]


if __name__ == "__main__":
    sys.exit(main())
