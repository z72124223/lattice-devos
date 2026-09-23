"""Install only the reviewed CPU choice experiment into ignored worktree storage."""
import hashlib
import ctypes
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time
import urllib.parse
import urllib.request
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
LOCAL = ROOT / '.lattice/life-harness/laya-local'
MANIFEST = HERE / 'laya-environment.json'
SDK = '010bacef009c855ccba814b51f7c8e1d38ab5e3f'
MODEL = 'b4a904d1a2a54c822b829e24291d4b8f280fe43e'
GIB = 1024**3


def request(url, method='GET'):
    return urllib.request.urlopen(urllib.request.Request(url, method=method,
        headers={'User-Agent': 'Mozilla/5.0 (compatible; LatticeOfflineCheck/1.0)'}), timeout=60)


def footprint():
    return sum(p.stat().st_size for p in LOCAL.rglob('*') if p.is_file())


def budget(reserve=0):
    used, free = footprint(), shutil.disk_usage(ROOT).free
    if used + reserve > 6 * GIB or free - reserve < 10 * GIB:
        raise RuntimeError(f'disk limit: used={used}, free={free}, reserve={reserve}')
    return {'project_bytes': used, 'disk_free_bytes': free}


def fetch_json(url):
    with request(url) as response:
        return json.load(response)


def isolated_env():
    env = {k: os.environ[k] for k in ('SYSTEMROOT', 'WINDIR', 'COMSPEC') if k in os.environ}
    env.update(PATH=str(Path(sys.executable).parent) + ';' + str(Path(env['SYSTEMROOT']) / 'System32'),
               TEMP=str(LOCAL / 'temp'), TMP=str(LOCAL / 'temp'), USERPROFILE=str(LOCAL / 'home'),
               PIP_CONFIG_FILE='NUL', PIP_DISABLE_PIP_VERSION_CHECK='1', PYTHONDONTWRITEBYTECODE='1')
    return env


def inspect():
    report = json.loads((LOCAL / 'pip-report.json').read_text(encoding='utf8'))
    wheels = []
    for item in report['install']:
        url = item['download_info']['url']
        assert urllib.parse.urlparse(url).hostname in ('files.pythonhosted.org', 'download-r2.pytorch.org')
        with request(url, 'HEAD') as response:
            size = int(response.headers['Content-Length'])
        wheels.append({'name': item['metadata']['name'], 'version': item['metadata']['version'], 'url': url,
                       'size': size, 'sha256': item['download_info']['archive_info']['hashes']['sha256'],
                       'file': urllib.parse.unquote(url.rsplit('/', 1)[1])})
    repo = fetch_json(f'https://huggingface.co/api/models/convaiinnovations/laya-multilingual/revision/{MODEL}?blobs=true')
    assert repo['sha'] == MODEL and repo['private'] is False and repo['gated'] is False
    model = []
    for item in repo['siblings']:
        if item['rfilename'] == '.gitattributes':
            continue
        model.append({'file': item['rfilename'], 'size': item['size'], 'git_blob': item['blobId'],
                      'sha256': item.get('lfs', {}).get('sha256'),
                      'url': f'https://huggingface.co/convaiinnovations/laya-multilingual/resolve/{MODEL}/' + item['rfilename']})
    tree = fetch_json(f'https://api.github.com/repos/NandhaKishorM/laya/git/trees/{SDK}?recursive=1')
    sdk = [{'file': item['path'], 'size': item['size'], 'git_blob': item['sha'],
            'url': f'https://raw.githubusercontent.com/NandhaKishorM/laya/{SDK}/' + item['path']}
           for item in tree['tree'] if item['type'] == 'blob' and
           (item['path'].startswith('laya/') or item['path'] in ('LICENSE', 'README.md', 'pyproject.toml'))]
    estimate = sum(w['size'] for w in wheels) * 10 + sum(f['size'] for f in model) * 2 + 256 * 1024**2
    initial = budget(estimate)
    data = {'schema_version': 1, 'sdk_revision': SDK, 'model_id': repo['id'], 'model_revision': MODEL,
            'encoder': {'upstream_id': 'jhu-clsp/mmBERT-base', 'artifact_revision': MODEL, 'separate_weights_download': False},
            'tokenizer_revision': MODEL, 'license': {'sdk': 'Apache-2.0', 'model_card': repo['cardData']['license']},
            'python': sys.version.split()[0], 'device': 'cpu', 'torch_dtype': 'float32',
            'limits': {'disk_bytes': 6 * GIB, 'minimum_free_bytes': 10 * GIB, 'memory_bytes': 8 * GIB, 'worker_seconds': 300},
            'pre_download': {**initial, 'conservative_additional_bytes': estimate,
                             'wheel_download_bytes': sum(w['size'] for w in wheels),
                             'model_download_bytes': sum(f['size'] for f in model),
                             'estimate_method': '10x compressed wheels + 2 model copies + 256MiB overhead'},
            'wheels': wheels, 'sdk_files': sdk, 'model_files': model}
    with MANIFEST.open('x', encoding='utf8', newline='\n') as out:
        json.dump(data, out, indent=2); out.write('\n')
    lines = [f"{w['name']} @ {w['url']} --hash=sha256:{w['sha256']}" for w in wheels]
    (HERE / 'laya-requirements.lock.txt').write_text('\n'.join(lines) + '\n', encoding='utf8', newline='\n')
    print(json.dumps(data['pre_download']))


def download(item, base):
    target = base / item['file']
    if target.exists():
        verify(item, target)
        return
    budget(item['size'])
    target.parent.mkdir(parents=True, exist_ok=True)
    with request(item['url']) as source, target.open('xb') as dest:
        count = 0
        while chunk := source.read(1024**2):
            count += len(chunk)
            if count > item['size']:
                raise RuntimeError('download exceeds frozen size')
            budget(len(chunk))
            dest.write(chunk)
    verify(item, target)


def verify(item, target):
    assert target.stat().st_size == item['size'], str(target)
    if item.get('sha256'):
        with target.open('rb') as stream:
            assert hashlib.file_digest(stream, 'sha256').hexdigest() == item['sha256'], str(target)
    else:
        content = target.read_bytes()
        assert hashlib.sha1(f'blob {len(content)}\0'.encode() + content).hexdigest() == item['git_blob'], str(target)


def run(command, name):
    log_path = LOCAL / (name + '.txt')
    number = 2
    while log_path.exists():
        log_path = LOCAL / f'{name}-{number}.txt'
        number += 1
    with log_path.open('x', encoding='utf8') as log:
        p = subprocess.Popen(command, env=isolated_env(), stdout=log, stderr=subprocess.STDOUT)
        start = time.monotonic()
        try:
            while p.poll() is None:
                budget(32 * 1024**2)
                if time.monotonic() - start > 300:
                    raise TimeoutError(name)
                time.sleep(0.5)
        except BaseException:
            p.kill(); p.wait(); raise
    if p.returncode:
        raise RuntimeError(f'{name} exit {p.returncode}; see ignored {log_path.name}')


def short_existing_path(source):
    # Existing NTFS alias only: no registry, PATH, directory move, or drive mapping.
    buffer = ctypes.create_unicode_buffer(32768)
    size = ctypes.windll.kernel32.GetShortPathNameW(str(source), buffer, len(buffer))
    if not size or size >= len(buffer):
        raise RuntimeError('An existing Windows short path is required for this deep worktree')
    short = Path(buffer.value)
    assert short.samefile(source)
    return short


def install():
    sys.path.insert(0, str(HERE))
    from laya_limits import apply_memory_limit
    limits = apply_memory_limit()
    data = json.loads(MANIFEST.read_text(encoding='utf8'))
    for group, folder in [('sdk_files', 'sdk'), ('model_files', 'snapshot'), ('wheels', 'wheels')]:
        for item in data[group]:
            download(item, LOCAL / folder)
    unpacked = sum(sum(z.file_size for z in zipfile.ZipFile(p).infolist()) for p in (LOCAL / 'wheels').glob('*.whl'))
    budget(int(unpacked * 1.3) + sum(f['size'] for f in data['model_files']) + 128 * 1024**2)
    venv = LOCAL / 'venv'
    if not venv.exists():
        run([sys.executable, '-I', '-m', 'venv', '--without-pip', str(venv)], 'venv-create')
    pip = [sys.executable, '-I', '-m', 'pip', '--isolated', '--disable-pip-version-check', '--python', str(short_existing_path(venv) / 'Scripts/python.exe'),
           'install', '--no-index', '--no-deps', '--no-cache-dir', '--no-compile']
    run(pip + [str(LOCAL / 'wheels' / w['file']) for w in data['wheels']], 'pip-install')
    run(pip + ['--no-build-isolation', str(LOCAL / 'sdk')], 'sdk-install')
    if not (LOCAL / 'working-model').exists():
        shutil.copytree(LOCAL / 'snapshot', LOCAL / 'working-model')
    result = {'status': 'installed_not_yet_loaded', **budget(), 'wheel_unpacked_bytes': unpacked, 'installer_memory_limits': limits}
    (HERE / 'results/laya-install.json').write_text(json.dumps(result, indent=2) + '\n', encoding='utf8', newline='\n')
    print(json.dumps(result))


if __name__ == '__main__':
    assert len(sys.argv) == 2 and sys.argv[1] in ('inspect', 'install')
    inspect() if sys.argv[1] == 'inspect' else install()
