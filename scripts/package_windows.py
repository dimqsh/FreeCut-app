#!/usr/bin/env python3
"""Package a Windows x64 release and its actual DLL dependency closure."""
import argparse
import hashlib
import json
import os
import shutil
import struct
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def imports(path):
    """Read ordinary and delay-load imports from PE32+ without external tools."""
    data = path.read_bytes()
    pe = struct.unpack_from('<I', data, 0x3C)[0]
    if data[:2] != b'MZ' or data[pe:pe + 4] != b'PE\0\0':
        raise ValueError(f'Not a PE executable: {path}')
    machine, count = struct.unpack_from('<HH', data, pe + 4)
    optional_size = struct.unpack_from('<H', data, pe + 20)[0]
    opt = pe + 24
    if machine != 0x8664 or struct.unpack_from('<H', data, opt)[0] != 0x20B:
        raise ValueError(f'Not an x64 PE32+ executable: {path}')
    image_base = struct.unpack_from('<Q', data, opt + 24)[0]
    sections = [struct.unpack_from('<8sIIII', data, opt + optional_size + 40 * n)
                for n in range(count)]

    def offset(rva):
        for _, virtual_size, address, raw_size, raw in sections:
            if address <= rva < address + max(virtual_size, raw_size):
                return raw + rva - address
        raise ValueError(f'Invalid import RVA in {path}: {rva:x}')

    names = set()
    directory_count = struct.unpack_from('<I', data, opt + 108)[0]
    for index, stride, name_offset in ((1, 20, 12), (13, 32, 4)):
        if directory_count <= index:
            continue
        rva, size = struct.unpack_from('<II', data, opt + 112 + 8 * index)
        if not rva:
            continue
        start = offset(rva)
        for pos in range(start, start + size, stride):
            if not any(data[pos:pos + stride]):
                break
            name_rva = struct.unpack_from('<I', data, pos + name_offset)[0]
            if index == 13 and not (struct.unpack_from('<I', data, pos)[0] & 1):
                name_rva -= image_base
            name = offset(name_rva)
            names.add(data[name:data.index(b'\0', name)].decode('ascii').lower())
    return names


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/x86_64-pc-windows-msvc/release/freecut.exe')
    parser.add_argument('--ffmpeg', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/windows-dist')
    args = parser.parse_args()
    version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    output = args.output.resolve()
    folder = output / f'FreeCut-{version}-windows-x64'
    archive = output / f'{folder.name}-portable.zip'
    if folder.exists() or archive.exists():
        parser.error('Output already exists; choose another --output directory.')
    folder.mkdir(parents=True)
    candidates = {p.name.lower(): p for p in (args.ffmpeg / 'bin').glob('*.dll')}
    pending = [args.binary.resolve()]
    bundled, system = {}, set()
    system_dir = Path(os.environ.get('SystemRoot', 'C:/Windows')) / 'System32'
    while pending:
        source = pending.pop()
        name = source.name.lower()
        if name in bundled:
            continue
        bundled[name] = source
        for dep in imports(source):
            if dep in candidates:
                pending.append(candidates[dep])
            elif dep.startswith(('api-ms-win-', 'ext-ms-win-')) or (system_dir / dep).is_file():
                # VC++ redist DLLs are not baseline Windows components.
                if dep.startswith(('vcruntime', 'msvcp', 'concrt')):
                    raise RuntimeError(f'Unbundled Visual C++ runtime: {dep}')
                system.add(dep)
            else:
                raise RuntimeError(f'Missing dependency: {source.name}: {dep}')
    for source in bundled.values():
        shutil.copy2(source, folder / source.name)
    licenses = folder / 'Licenses'
    licenses.mkdir()
    shutil.copy2(ROOT / 'LICENSE', licenses / 'FreeCut-GPL-3.0.txt')
    shutil.copy2(ROOT / 'assets/ASSETS.md', licenses / 'ASSETS.md')
    for p in (ROOT / 'assets/fonts').glob('*.txt'):
        shutil.copy2(p, licenses / p.name)
    for name in ('LICENSE.txt', 'LICENSE', 'LICENSE.md', 'COPYING.GPLv3', 'COPYING.LGPLv3'):
        p = args.ffmpeg / name
        if p.is_file():
            shutil.copy2(p, licenses / ('FFmpeg-' + p.name))
    if (args.ffmpeg / 'doc').is_dir():
        shutil.copytree(args.ffmpeg / 'doc', licenses / 'FFmpeg-build')
    (folder / 'FreeCut-CPU.cmd').write_bytes(
        b'@echo off\r\nsetlocal\r\ncd /d "%~dp0"\r\n'
        b'set "FREECUT_LOW_MEMORY=1"\r\n'
        b'if not exist "%LOCALAPPDATA%\\FreeCut" mkdir "%LOCALAPPDATA%\\FreeCut"\r\n'
        b'start "" /wait "%~dp0freecut.exe" --software --low-memory '
        b'1>"%LOCALAPPDATA%\\FreeCut\\cpu-start.log" 2>&1\r\n'
        b'if errorlevel 1 (type "%LOCALAPPDATA%\\FreeCut\\cpu-start.log" & pause)\r\n')
    spec = json.loads((ROOT / 'scripts/windows-ffmpeg.json').read_text())
    (folder / 'README.txt').write_text(
        'FreeCut ' + version + '\nWindows 10 22H2 / Windows 11, x64.\n'
        'Extract the whole folder. Run freecut.exe. No administrator rights required.\n'
        'CPU-only / 2 GB RAM: run FreeCut-CPU.cmd (WARP, small caches).\n'
        'The installer provides a FreeCut (CPU) shortcut with the same options.\n'
        '2 GB is a target, not a guarantee for arbitrary projects or 4K editing.\n'
        'Windows 7/8, 32-bit Windows and old physical PCs have not been validated.\n'
        'User media, exports and caches need additional disk space.\n'
        'Unsigned test build. See Licenses and SOURCE.txt.\n', encoding='utf-8')
    (folder / 'SOURCE.txt').write_text(
        'FreeCut source: https://github.com/arystambek-dimash/FreeCut-app\n'
        'Build scripts: scripts/ and .github/workflows/windows-package.yml\n'
        'FFmpeg distribution and build recipes: ' + spec['provider'] + '\n'
        'Pinned FFmpeg archive: ' + spec['url'] + '\n'
        'SHA-256: ' + spec['sha256'] + '\n', encoding='utf-8')
    manifest = {'version': version, 'architecture': 'x86_64', 'minimum_windows': '10 22H2',
                'cpu_target': 'x86-64 baseline; no mandatory AVX/AVX2 in Rust code',
                'software_ui': 'Direct3D 12 WARP; FXC', 'ffmpeg': spec,
                'system_dlls': sorted(system),
                'files': [{'name': p.name, 'bytes': p.stat().st_size} for p in bundled.values()]}
    manifest['installed_logical_bytes'] = sum(p.stat().st_size for p in folder.rglob('*') if p.is_file())
    (folder / 'build-info.json').write_text(json.dumps(manifest, indent=2) + '\n')
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for p in sorted(folder.rglob('*')):
            if p.is_file():
                z.write(p, p.relative_to(output))
    manifest['zip_bytes'] = archive.stat().st_size
    with archive.open('rb') as f:
        digest = hashlib.file_digest(f, 'sha256').hexdigest()
    (output / (archive.name + '.sha256')).write_text(f'{digest}  {archive.name}\n')
    (output / 'package-info.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(json.dumps(manifest, indent=2))


if __name__ == '__main__':
    main()
