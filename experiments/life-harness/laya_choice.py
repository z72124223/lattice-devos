"""One fixed local CPU choice path; receives gated data, never executes case commands."""
import hashlib
import importlib.metadata
import json
from pathlib import Path
import shutil
import sys
import time

HERE = Path(__file__).resolve().parent
WORKER_STARTED = time.perf_counter()
sys.path.insert(0, str(HERE))  # Explicit trusted experiment path when launched with -I.
from laya_limits import apply_memory_limit, deny_network, get_peak_rss


def token_budget(agent, request):
    from laya.common import render_options, serialize_state, build_sequence
    definition = request['questions']['diagnosis']
    agent._check_question('diagnosis', definition)
    assert definition['type'] == 'choice'
    q = agent._to_internal(definition)
    tok = agent.tok
    count = lambda value: len(tok(value.replace(tok.mask_token, ' '), add_special_tokens=False)['input_ids'])
    option_counts = [count(' ' + option) for option in render_options(q)]
    options = sum(n + 1 for n in option_counts)
    instruction = count('choice question: ' + q['ins'])
    state = count(serialize_state(request['state']))
    maximum, head = agent.cfg['max_len'], agent.cfg['head_max_len']
    checks = {'options': option_counts, 'instruction': instruction, 'state': state,
              'total': instruction + options + state + 4, 'maximum': maximum, 'head_maximum': head}
    if any(n > 48 for n in option_counts) or options > head - 16 or instruction > head - options or checks['total'] > maximum:
        return checks, False
    ids, markers = build_sequence(tok, request['state'], q, max_len=maximum, head_max_len=head)
    assert len(ids) == checks['total'] and len(markers) == len(option_counts), 'unexpected SDK truncation'
    return checks, True


def offline_loaders(model_dir):
    import huggingface_hub
    from transformers import AutoConfig, AutoTokenizer
    def forbidden(*args, **kwargs):
        raise PermissionError('Hub downloads and child processes are forbidden during inference')
    huggingface_hub.snapshot_download = forbidden
    huggingface_hub.hf_hub_download = forbidden
    for cls in (AutoConfig, AutoTokenizer):
        original = cls.from_pretrained
        def local_only(name, *args, _original=original, **kwargs):
            source = Path(name)
            if not source.is_absolute() or not source.is_dir() or not source.is_relative_to(model_dir):
                raise PermissionError('Only the verified local model subdirectories may be loaded')
            kwargs.update(local_files_only=True, trust_remote_code=False, token=False)
            return _original(name, *args, **kwargs)
        cls.from_pretrained = local_only


def main():
    started = time.perf_counter()
    assert sys.flags.no_site and 'site' not in sys.modules, 'Start with -I -S -B; no startup .pth execution'
    limits = apply_memory_limit()
    deny_network()
    def no_children(event, args):
        if event in ('subprocess.Popen', 'os.system', 'os.posix_spawn', 'os.spawn'):
            raise PermissionError('Child processes are forbidden during inference')
    sys.addaudithook(no_children)
    assert len(sys.argv) == 2
    payload = json.loads(Path(sys.argv[1]).read_text(encoding='utf8'))
    assert payload['mode'] in ('smoke', 'advisories')
    manifest = json.loads((HERE / 'laya-environment.json').read_text(encoding='utf8'))
    assert sys.version.split()[0] == manifest['python']
    local = HERE.parent.parent / '.lattice/life-harness/laya-local'
    site_packages = (local / 'venv/Lib/site-packages').resolve()
    assert Path(sys.executable).resolve() == (local / 'venv/Scripts/python.exe').resolve()
    assert site_packages.is_dir()
    sys.path.append(str(site_packages))  # Explicit packages only; never execute .pth files.
    model_dir = (local / 'working-model').resolve()
    from laya_setup import verify
    for item in manifest['model_files']:
        verify(item, local / 'snapshot' / item['file'])
        if item['file'] != 'tokenizer/tokenizer_config.json':
            verify(item, model_dir / item['file'])
    # The SDK rewrites this small config. Preserve the original snapshot and reset only its working copy.
    shutil.copyfile(local / 'snapshot/tokenizer/tokenizer_config.json', model_dir / 'tokenizer/tokenizer_config.json')
    versions = {w['name']: importlib.metadata.version(w['name']) for w in manifest['wheels']}
    for wheel in manifest['wheels']:
        assert versions[wheel['name']] == wheel['version']
    import torch
    assert torch.version.cuda is None and torch.get_default_dtype() == torch.float32
    torch.set_num_threads(4)
    torch.set_num_interop_threads(1)
    torch.manual_seed(0)
    offline_loaders(model_dir)
    try:
        importlib.metadata.distribution('laya')
    except importlib.metadata.PackageNotFoundError:
        pass
    else:
        raise RuntimeError('This acceptance path requires the recorded uninstalled SDK source tree')
    from laya_source import load_verified_source, verify_loaded_modules
    sdk_dir = (local / 'sdk').resolve()
    laya, sdk_version = load_verified_source(sdk_dir, manifest)
    load_started = time.perf_counter()
    agent = laya.load(str(model_dir), device='cpu')
    load_seconds = time.perf_counter() - load_started
    assert agent.device.type == 'cpu' and agent.dtype == torch.float32
    assert {str(p.dtype) for p in agent.model.parameters()} == {'torch.float32'}
    assert agent.model.encoder.config.reference_compile is False
    records = []
    for record in payload['records']:
        output = {k: v for k, v in record.items() if k != 'request'}
        output.update(model_called=False, adopted=False, calibrated_score=None, raw_model_output=None)
        if record['gate']['decision'] != 'request':
            assert record['request'] is None
            output.update(decision=record['gate']['decision'], reason=record['gate']['reason'], inference_seconds=0)
        else:
            request = record['request']
            allowed = set(record['candidates']) | {'none_applicable', 'insufficient_information'}
            assert record['candidates'] and set(request['questions']['diagnosis']['criteria']) == allowed
            tokens, fits = token_budget(agent, request)
            output['token_counts'] = tokens
            if not fits:
                output.update(decision='abstain', reason='input_would_be_truncated', inference_seconds=0)
            else:
                before = time.perf_counter()
                result = agent.predict(request['state'], request['questions'])
                seconds = time.perf_counter() - before
                choice = result['answers']['diagnosis']['choice']
                assert choice in allowed
                output.update(model_called=True, raw_model_output=result, inference_seconds=seconds,
                              decision='selected' if choice in record['candidates'] else 'abstain',
                              selected_procedure=choice if choice in record['candidates'] else None,
                              reason=None if choice in record['candidates'] else choice)
        records.append(output)
    verify_loaded_modules(sdk_dir, manifest)
    result = {'status': 'completed', 'mode': payload['mode'], 'sdk_revision': manifest['sdk_revision'],
              'sdk_version': sdk_version, 'sdk_load_mode': 'verified_source_tree', 'sdk_distribution_installed': False,
              'model_revision': manifest['model_revision'], 'encoder_revision': manifest['encoder']['artifact_revision'],
              'tokenizer_revision': manifest['tokenizer_revision'], 'versions': versions, 'python': sys.version.split()[0],
              'device': 'cpu', 'parameter_dtype': 'float32', 'cuda_version': torch.version.cuda, 'cpu_threads': torch.get_num_threads(),
              'load_seconds': load_seconds, 'worker_seconds': time.perf_counter() - started, 'peak_rss_bytes': get_peak_rss(),
              'memory_limits': limits, 'network_control': 'Python socket audit + offline Hub + local-only loaders + no subprocesses; not an OS firewall',
              'working_tokenizer_config_sha256': hashlib.sha256((model_dir / 'tokenizer/tokenizer_config.json').read_bytes()).hexdigest(),
              'score_interpretation': 'SDK softmax outputs and entropy/action scores, uncalibrated for this task; no adoption threshold',
              'records': records}
    if payload['mode'] == 'smoke':
        assert len(records) == 1 and records[0]['source_kind'] == 'synthetic' and records[0]['model_called']
    print(json.dumps(result, ensure_ascii=True), flush=True)


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'status': 'failed', 'error_type': type(error).__name__, 'error': str(error),
                          'worker_seconds': time.perf_counter() - WORKER_STARTED,
                          'peak_rss_bytes': get_peak_rss()}, ensure_ascii=True), flush=True)
        raise
