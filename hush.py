#!/usr/bin/env python3
"""hush — local push-to-talk dictation. Hold fn, talk, release to paste."""
import os
import subprocess
import sys
import time

import numpy as np
import sounddevice as sd
import Quartz
from faster_whisper import WhisperModel

FN_FLAG = Quartz.kCGEventFlagMaskSecondaryFn
SAMPLE_RATE = 16000
MIN_DURATION_SEC = 0.3
MODEL_SIZE = os.environ.get("WHISPER_MODEL", "small.en")
MODEL_DIR = os.path.expanduser("~/.cache/hush/models")

START_SOUND = "/System/Library/Sounds/Tink.aiff"
STOP_SOUND = "/System/Library/Sounds/Pop.aiff"


class PasteError(RuntimeError):
    pass


def play(path: str) -> None:
    subprocess.Popen(
        ["afplay", path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


class Dictator:
    def __init__(self) -> None:
        os.makedirs(MODEL_DIR, exist_ok=True)
        print(f"[hush] loading model {MODEL_SIZE}...", flush=True)
        self.model = WhisperModel(
            MODEL_SIZE,
            device="cpu",
            compute_type="int8",
            download_root=MODEL_DIR,
        )
        self.recording = False
        self.chunks: list[np.ndarray] = []
        self.stream: sd.InputStream | None = None
        print("[hush] ready. hold fn to dictate.", flush=True)

    def _on_audio(self, indata, frames, time_info, status) -> None:
        if self.recording:
            self.chunks.append(indata.copy())

    def start(self) -> None:
        if self.recording:
            return
        play(START_SOUND)
        self.recording = True
        self.chunks = []
        self.stream = sd.InputStream(
            samplerate=SAMPLE_RATE,
            channels=1,
            dtype="float32",
            callback=self._on_audio,
        )
        self.stream.start()

    def stop(self) -> None:
        if not self.recording:
            return
        self.recording = False
        if self.stream is not None:
            self.stream.stop()
            self.stream.close()
            self.stream = None
        play(STOP_SOUND)

        if not self.chunks:
            return
        audio = np.concatenate(self.chunks).flatten()
        if len(audio) < SAMPLE_RATE * MIN_DURATION_SEC:
            print("[hush] too short, skipping", flush=True)
            return

        t0 = time.time()
        segments, _ = self.model.transcribe(
            audio,
            beam_size=1,
            language="en",
            condition_on_previous_text=False,
        )
        text = "".join(seg.text for seg in segments).strip()
        elapsed = time.time() - t0
        if not text:
            print(f"[hush] no speech detected ({elapsed:.1f}s)", flush=True)
            return
        print(f"[hush] ({elapsed:.1f}s) {text}", flush=True)
        try:
            paste(text)
        except PasteError as e:
            print(f"[hush] paste failed: {e}", flush=True)


def paste(text: str) -> None:
    try:
        prev = subprocess.run(
            ["pbpaste"], capture_output=True, check=True
        ).stdout
    except Exception:
        prev = b""

    subprocess.run(["pbcopy"], input=text.encode("utf-8"), check=True)
    try:
        result = subprocess.run(
            [
                "osascript",
                "-e",
                'tell application "System Events" to keystroke "v" using command down',
            ],
            capture_output=True,
        )
        if result.returncode != 0:
            stderr = result.stderr.decode("utf-8", errors="replace").strip()
            if "1002" in stderr or "not allowed to send keystrokes" in stderr:
                raise PasteError(
                    "Accessibility permission missing. "
                    "Grant in System Settings → Privacy & Security → Accessibility."
                )
            raise PasteError(stderr or f"osascript exited {result.returncode}")
        time.sleep(0.25)
    finally:
        subprocess.run(["pbcopy"], input=prev, check=True)


def main() -> int:
    if not Quartz.CGPreflightListenEventAccess():
        Quartz.CGRequestListenEventAccess()
        if not Quartz.CGPreflightListenEventAccess():
            print(
                "[hush] Input Monitoring not granted. "
                "Add this binary in System Settings → Privacy & Security → Input Monitoring, "
                "then relaunch.",
                flush=True,
            )
            return 1

    dictator = Dictator()
    fn_down = False

    def callback(proxy, event_type, event, refcon):
        nonlocal fn_down
        flags = Quartz.CGEventGetFlags(event)
        now = bool(flags & FN_FLAG)
        if now and not fn_down:
            fn_down = True
            dictator.start()
        elif not now and fn_down:
            fn_down = False
            dictator.stop()
        return event

    mask = Quartz.CGEventMaskBit(Quartz.kCGEventFlagsChanged)
    tap = Quartz.CGEventTapCreate(
        Quartz.kCGSessionEventTap,
        Quartz.kCGHeadInsertEventTap,
        Quartz.kCGEventTapOptionListenOnly,
        mask,
        callback,
        None,
    )
    if tap is None or not Quartz.CGEventTapIsEnabled(tap):
        print(
            "[hush] event tap unavailable. "
            "Ensure this binary has Input Monitoring permission and relaunch.",
            flush=True,
        )
        return 1

    source = Quartz.CFMachPortCreateRunLoopSource(None, tap, 0)
    Quartz.CFRunLoopAddSource(
        Quartz.CFRunLoopGetCurrent(), source, Quartz.kCFRunLoopCommonModes
    )
    Quartz.CGEventTapEnable(tap, True)
    Quartz.CFRunLoopRun()
    return 0


if __name__ == "__main__":
    sys.exit(main())
