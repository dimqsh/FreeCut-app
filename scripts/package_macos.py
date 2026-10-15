#!/usr/bin/env python3
"""Build a local macOS DMG with its non-system dylibs. Uses Python stdlib only."""

import argparse
import hashlib
import json
import platform
import plistlib
import re
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def run(*args):
    return subprocess.check_output(args, text=True, cwd=ROOT).strip()


def dependencies(path):
    return [
        line.strip().split(" (compatibility", 1)[0]
        for line in run("otool", "-L", str(path)).splitlines()[1:]
    ]


def system_library(path):
    return path.startswith(("/System/Library/", "/usr/lib/"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/dist")
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("Run this packager on macOS with Xcode command-line tools.")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    arch = platform.machine()
    stem = f"FreeCut-{version}-macos-{arch}"
    app_dest, dmg_dest = output / "FreeCut.app", output / f"{stem}.dmg"
    manifest_dest = output / f"{stem}.json"
    for path in (app_dest, dmg_dest, manifest_dest):
        if path.exists():
            parser.error(f"Output already exists: {path}. Choose another --output directory.")
    subprocess.run(["cargo", "build", "-p", "freecut-app", "--release", "--locked"],
                   cwd=ROOT, check=True)
    binary = (ROOT / "target/release/freecut").resolve()
    graph = {}
    pending = [binary]
    while pending:
        source = pending.pop()
        if source in graph:
            continue
        graph[source] = dependencies(source)
        for dep in graph[source]:
            if system_library(dep):
                continue
            if not dep.startswith("/"):
                raise RuntimeError(f"Cannot resolve non-absolute build dependency: {dep}")
            library = Path(dep).resolve(strict=True)
            if library != source:
                pending.append(library)
    libraries = sorted(path for path in graph if path != binary)
    if len({p.name for p in libraries}) != len(libraries):
        raise RuntimeError("Two different libraries have the same file name.")
    # cpal's process-tap imports also require 14.2, beyond the Rust Mach-O minimum.
    minimums = [(14, 2)]
    for source in graph:
        if arch not in run("lipo", "-archs", str(source)).split():
            raise RuntimeError(f"Library has no {arch} slice: {source}")
        load_commands = run("otool", "-l", str(source))
        minimums.extend(tuple(map(int, v.split(".")))
                        for v in re.findall(r"\bminos (\S+)", load_commands))
    minimum = ".".join(map(str, max(minimums)))
    with tempfile.TemporaryDirectory(prefix=".freecut-package-", dir=output) as temporary:
        work = Path(temporary)
        stage = work / "image"
        app = stage / "FreeCut.app"
        contents = app / "Contents"
        executable = contents / "MacOS/freecut"
        frameworks = contents / "Frameworks"
        licenses = contents / "Resources/Licenses"
        for directory in (executable.parent, frameworks, licenses):
            directory.mkdir(parents=True, exist_ok=True)
        shutil.copy2(binary, executable)
        shutil.copy2(ROOT / "LICENSE", licenses / "FreeCut-GPL-3.0.txt")
        shutil.copy2(ROOT / "assets/ASSETS.md", licenses / "ASSETS.md")
        for license_file in (ROOT / "assets/fonts").glob("*.txt"):
            shutil.copy2(license_file, licenses / license_file.name)
        copied = {binary: executable}
        for source in libraries:
            destination = frameworks / source.name
            shutil.copy2(source, destination)
            destination.chmod(0o755)
            copied[source] = destination
            if "/Cellar/" in str(source):
                parts = source.parts
                cellar = parts.index("Cellar")
                formula_root = Path(*parts[:cellar + 3])
                formula = parts[cellar + 1]
                for license_file in formula_root.iterdir():
                    if license_file.is_file() and license_file.name.upper().startswith(
                        ("LICENSE", "COPYING", "COPYRIGHT", "NOTICE")
                    ):
                        folder = licenses / formula
                        folder.mkdir(exist_ok=True)
                        shutil.copy2(license_file, folder / license_file.name)
        for source, destination in copied.items():
            prefix = "@executable_path/../Frameworks/" if source == binary else "@loader_path/"
            for dep in graph[source]:
                if system_library(dep) or Path(dep).resolve() == source:
                    continue
                replacement = prefix + copied[Path(dep).resolve()].name
                run("install_name_tool", "-change", dep, replacement, str(destination))
            if source != binary:
                run("install_name_tool", "-id", "@rpath/" + destination.name, str(destination))
            for dep in dependencies(destination):
                if system_library(dep) or dep == "@rpath/" + destination.name:
                    continue
                if dep.startswith("@executable_path/"):
                    resolved = executable.parent / dep.removeprefix("@executable_path/")
                elif dep.startswith("@loader_path/"):
                    resolved = destination.parent / dep.removeprefix("@loader_path/")
                else:
                    raise RuntimeError(f"External dependency remains: {destination}: {dep}")
                if not resolved.is_file() or not resolved.resolve().is_relative_to(app.resolve()):
                    raise RuntimeError(f"Missing bundled dependency: {dep}")
        info = {
            "CFBundleName": "FreeCut", "CFBundleDisplayName": "FreeCut",
            "CFBundleIdentifier": "org.freecut.FreeCut", "CFBundleExecutable": "freecut",
            "CFBundlePackageType": "APPL", "CFBundleInfoDictionaryVersion": "6.0",
            "CFBundleShortVersionString": version, "CFBundleVersion": version,
            "LSMinimumSystemVersion": minimum, "NSHighResolutionCapable": True,
            "NSMicrophoneUsageDescription": "Record voiceovers in your FreeCut projects.",
        }
        (contents / "Info.plist").write_bytes(plistlib.dumps(info))
        for source in libraries:
            run("codesign", "--force", "--sign", "-", "--timestamp=none", str(copied[source]))
        run("codesign", "--force", "--sign", "-", "--timestamp=none", str(app))
        run("codesign", "--verify", "--deep", "--strict", str(app))
        (stage / "Applications").symlink_to("/Applications")
        dmg = work / f"{stem}.dmg"
        run("hdiutil", "create", "-volname", "FreeCut", "-srcfolder", str(stage),
            "-format", "UDZO", "-fs", "HFS+", str(dmg))
        run("hdiutil", "verify", str(dmg))
        with dmg.open("rb") as stream:
            checksum = hashlib.file_digest(stream, "sha256").hexdigest()
        manifest = {
            "version": version, "architecture": arch, "minimum_macos": minimum,
            "signing": "ad-hoc; local development bundle; not notarized",
            "binary_bytes": executable.stat().st_size,
            "bundled_libraries": [{"name": copied[p].name, "bytes": copied[p].stat().st_size}
                                  for p in libraries],
            "installed_logical_bytes": sum(p.stat().st_size for p in app.rglob("*") if p.is_file()),
            "installed_allocated_kib": int(run("du", "-sk", str(app)).split()[0]),
            "dmg_bytes": dmg.stat().st_size,
            "dmg_sha256": checksum,
            "dependency_check": "Only bundled libraries and macOS system libraries",
        }
        shutil.move(str(app), app_dest)
        shutil.move(str(dmg), dmg_dest)
        manifest_dest.write_text(json.dumps(manifest, indent=2) + "\n")
        print(json.dumps({"app": str(app_dest), "dmg": str(dmg_dest), **manifest}, indent=2))


if __name__ == "__main__":
    main()
