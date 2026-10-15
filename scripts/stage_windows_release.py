#!/usr/bin/env python3
"""Stage verified Windows artifacts as a draft. Never publishes a Release."""
import argparse
import hashlib
import json
import subprocess
import tempfile
from pathlib import Path


def gh(*args):
    return subprocess.check_output(['gh', *args], text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-id', required=True, type=int)
    parser.add_argument('--artifacts', required=True, type=Path)
    parser.add_argument('--repo', required=True)
    args = parser.parse_args()
    run = json.loads(gh('api', f'repos/{args.repo}/actions/runs/{args.run_id}'))
    if run['conclusion'] != 'success' or run['name'] != 'Windows package':
        raise RuntimeError('Only a successful Windows package run can become a draft Release')
    folder = args.artifacts / 'windows-dist'
    info = json.loads((folder / 'package-info.json').read_text())
    if info['source_commit'] != run['head_sha']:
        raise RuntimeError('Package source commit does not match its build run')
    checks = {}
    for kind in ('portable', 'installed'):
        p = args.artifacts / 'windows-check' / f'{kind}-cpu-result.json'
        result = json.loads(p.read_text(encoding='utf-8-sig'))
        if not result['normal_exit'] or not result['window_created']:
            raise RuntimeError(f'{kind}: normal GUI startup/exit was not verified')
        if 'Microsoft Basic Render Driver' not in result['adapter']:
            raise RuntimeError(f'{kind}: WARP was not verified')
        if not (0 < result['peak_committed_bytes'] <= result['memory_limit_bytes'] <= 1536 * 2**20):
            raise RuntimeError(f'{kind}: unexpected memory limit or peak')
        checks[kind] = result
    tag = 'v' + info['version'] + '-windows-preview'
    notes = f'''Preliminary Windows x64 build. Target: Windows 10 22H2 and Windows 11.

Download the setup EXE for a per-user installation, or extract the entire portable ZIP.
For a CPU-only / low-memory run, use **FreeCut (CPU)** in the Start menu or **FreeCut-CPU.cmd** in the portable folder.

The UI uses Windows WARP in CPU mode; no hardware GPU is selected. Small cache and decoder budgets are enabled. The installer and portable application both passed GUI startup and normal exit under a 1536 MiB Windows Job Object process-commit limit.

This limit applies to the application, not the whole OS. The test used a hosted Windows Server runner, not an old physical 2 GB PC. Windows 7/8 and 32-bit Windows are not supported by this package. Arbitrary projects and 4K workloads are not guaranteed to fit 2 GB. Unsigned preview build.

Installed payload: {info['installed_logical_bytes'] / 1e6:.1f} MB. Portable ZIP: {info['zip_bytes'] / 1e6:.1f} MB.
WARP process-commit peak: portable {checks['portable']['peak_committed_bytes'] / 2**20:.1f} MiB; installed {checks['installed']['peak_committed_bytes'] / 2**20:.1f} MiB.

Source commit: {info['source_commit']}
Build and tests: {run['html_url']}
FFmpeg build recipes: {info['ffmpeg']['source']}

SHA-256 files, package provenance and CPU-check results are attached.
'''
    assets = sorted(folder.glob('*setup.exe')) + sorted(folder.glob('*portable.zip'))
    if len(assets) != 2:
        raise RuntimeError('Exactly one installer and one portable ZIP are required')
    assets += sorted(folder.glob('*.sha256')) + [folder / 'package-info.json']
    with tempfile.TemporaryDirectory(prefix='freecut-release-') as temp:
        temp = Path(temp)
        (temp / 'notes.md').write_text(notes)
        (temp / 'Windows-CPU-checks.json').write_text(json.dumps(checks, indent=2) + '\n')
        assets.append(temp / 'Windows-CPU-checks.json')
        existing = subprocess.run(['gh', 'release', 'view', tag, '--repo', args.repo,
                                   '--json', 'isDraft,targetCommitish,assets'], capture_output=True, text=True)
        uploaded = {}
        if existing.returncode == 0:
            release = json.loads(existing.stdout)
            if not release['isDraft'] or release['targetCommitish'] != info['source_commit']:
                raise RuntimeError('Refusing to modify a published or different release')
            uploaded = {a['name']: a.get('digest') for a in release['assets']}
        else:
            gh('release', 'create', tag, '--repo', args.repo, '--draft', '--prerelease',
               '--target', info['source_commit'], '--title', f"FreeCut {info['version']} — Windows CPU preview",
               '--notes-file', str(temp / 'notes.md'))
        for asset in assets:
            with asset.open('rb') as stream:
                digest = 'sha256:' + hashlib.file_digest(stream, 'sha256').hexdigest()
            if asset.name in uploaded:
                if uploaded[asset.name] != digest:
                    raise RuntimeError(f'Conflicting existing release asset: {asset.name}')
                continue
            gh('release', 'upload', tag, str(asset), '--repo', args.repo)
        print(gh('release', 'view', tag, '--repo', args.repo, '--json', 'url,isDraft,assets'))


if __name__ == '__main__':
    main()
