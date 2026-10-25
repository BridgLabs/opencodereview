import argparse
import concurrent.futures
import importlib.util
import json
import pathlib
import select
import subprocess
import tempfile
import time
import uuid

spec = importlib.util.spec_from_file_location("single_diagnostic", pathlib.Path(__file__).with_name("run-real-review-diagnostic.py"))
single = importlib.util.module_from_spec(spec)
spec.loader.exec_module(single)
docker = single.docker

BARRIER = """
import time
(root / 'preflight-ready').touch(exist_ok=False)
deadline = time.monotonic() + 180
while not pathlib.Path('/barrier/go').exists():
    if time.monotonic() > deadline:
        raise RuntimeError('Diagnostic barrier expired before inference')
    time.sleep(0.01)
"""
CLIENT = single.CLIENT.replace("with connect() as stream:\n    install(stream)\n    with (root / 'execution-once.json')", BARRIER + "\nwith connect() as stream:\n    install(stream)\n    with (root / 'execution-once.json')")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--image', required=True)
    parser.add_argument('--test-image', required=True)
    parser.add_argument('--repository', required=True)
    parser.add_argument('--pr', required=True, type=int)
    parser.add_argument('--run-id', required=True, type=int)
    parser.add_argument('--authorize-three-model-executions', required=True, action='store_true')
    args = parser.parse_args()
    root = pathlib.Path(tempfile.mkdtemp(prefix=f'ocr-run-{args.run_id}-parallel-'))
    print(json.dumps({'diagnosticDirectory': str(root), 'maximumExecutions': 3}), flush=True)
    for chunk in [1, 2, 3]:
        directory = root / f'chunk-{chunk}'
        result = subprocess.run(['python3', str(pathlib.Path(__file__).with_name('check-review-context.py')), '--repository', args.repository, '--pr', str(args.pr), '--run-id', str(args.run_id), '--image', args.test_image, '--export-chunk', str(chunk), '--export-directory', str(directory)], capture_output=True, text=True, timeout=300)
        if result.returncode:
            raise RuntimeError('Offline stage export failed: ' + result.stderr.strip())
        packet = json.loads((directory / 'stage.json').read_text())
        assert packet['runId'] == args.run_id and packet['chunk'] == chunk
    packets = [json.loads((root / f'chunk-{chunk}' / 'stage.json').read_text()) for chunk in [1, 2, 3]]
    assert len({packet['engineVersion'] for packet in packets}) == 1
    assert len({packet['descriptor']['id'] for packet in packets}) == 1
    assert len({packet['request']['model'] for packet in packets}) == 1
    engine = docker('run', '--rm', '--network', 'none', '--read-only', args.image, 'engine-version').stdout.strip()
    assert engine == packets[0]['engineVersion'], 'Executor mismatch'
    assert CLIENT != single.CLIENT, 'Missing start barrier'
    barrier = root / 'barrier'
    barrier.mkdir(mode=0o700)
    prefix = 'ocr-parallel-diagnostic-' + uuid.uuid4().hex[:12]
    containers, networks, volumes, collectors, streams, clients = [], [], [], [], [], []
    try:
        outbound = prefix + '-outbound'
        docker('network', 'create', outbound)
        networks.append(outbound)
        common = ['--init', '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--pids-limit', '128', '--cpus', '1', '--memory', '2g', '--memory-swap', '2g', '--tmpfs', '/data:size=1m,mode=0700', '--tmpfs', '/tmp:size=64m,mode=1777']
        proxy = prefix + '-proxy'
        docker('create', '--name', proxy, *common, '--network', outbound, '--user', '10001:10001', '--cpus', '0.25', '--memory', '128m', '--memory-swap', '128m', args.image, 'egress')
        containers.append(proxy)
        for chunk in [1, 2, 3]:
            internal, ipc = f'{prefix}-network-{chunk}', f'{prefix}-ipc-{chunk}'
            docker('network', 'create', '--internal', internal)
            networks.append(internal)
            docker('network', 'connect', '--alias', 'diagnostic-egress', internal, proxy)
            docker('volume', 'create', ipc)
            volumes.append(ipc)
            runner = f'{prefix}-runner-{chunk}'
            docker('create', '--name', runner, '--hostname', runner, *common, '--network', internal, '--cap-add', 'SETUID', '--cap-add', 'SETGID', '--cap-add', 'CHOWN', '--cap-add', 'DAC_OVERRIDE', '--cap-add', 'KILL', '--mount', f'type=volume,source={ipc},target=/ipc', '--mount', 'type=volume,source=opencodereview_review-claude-auth,target=/claude-auth,readonly', '-e', 'OCR_RUNNER_CLAUDE_TOKEN=/claude-auth/app-managed/claude/oauth-token', '-e', 'OCR_RUNNER_PROXY=http://diagnostic-egress:8790', '--tmpfs', '/work:size=16m,mode=0755', '--tmpfs', '/evidence:size=512m,mode=0755', '--tmpfs', '/output:size=48m,mode=0777', '--tmpfs', '/scratch:size=128m,mode=0755', args.image, 'runner')
            containers.append(runner)
            docker('start', runner)
            for _ in range(40):
                if docker('exec', runner, 'test', '-S', f'/ipc/{runner}.sock', check=False).returncode == 0:
                    break
                time.sleep(0.1)
            else:
                raise RuntimeError('Runner IPC unavailable')
            output = (root / f'chunk-{chunk}' / 'cli-console.json').open('x')
            (root / f'chunk-{chunk}' / 'cli-console.json').chmod(0o600)
            streams.append(output)
            collector = subprocess.Popen(['docker', 'exec', runner, 'perl', '-e', single.COLLECTOR], stdout=output, stderr=subprocess.PIPE)
            collectors.append(collector)
            if not select.select([collector.stderr], [], [], 5)[0] or collector.stderr.readline() != b'CLI_CAPTURE_READY\n':
                raise RuntimeError('Collector unavailable before inference')
            client = f'{prefix}-client-{chunk}'
            docker('create', '--name', client, '--network', 'none', '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--memory', '256m', '--cpus', '0.5', '--tmpfs', '/tmp:size=16m', '--mount', f'type=volume,source={ipc},target=/ipc,readonly', '--mount', f'type=bind,source={root / f"chunk-{chunk}"},target=/diagnostic', '--mount', f'type=bind,source={barrier},target=/barrier,readonly', '-e', f'OCR_DIAGNOSTIC_RUNNER_SOCKET=/ipc/{runner}.sock', 'python:3.12-slim', 'python3', '-u', '-c', CLIENT)
            clients.append(client)
            containers.append(client)
        docker('start', proxy)
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as pool:
            futures = {pool.submit(docker, 'start', '-a', client, check=False, timeout=1100): client for client in clients}
            deadline = time.monotonic() + 180
            while not all((root / f'chunk-{chunk}' / 'preflight-ready').exists() for chunk in [1, 2, 3]):
                if time.monotonic() > deadline or any(future.done() for future in futures):
                    for client in clients:
                        docker('stop', '--time', '2', client, check=False)
                    raise RuntimeError('Preflight failed before inference barrier')
                time.sleep(0.1)
            print(json.dumps({'phase': 'preflights-ready', 'stages': 3, 'modelCalls': 0}), flush=True)
            (barrier / 'go').touch(exist_ok=False)
            failed = False
            for future in concurrent.futures.as_completed(futures):
                client = futures[future]
                result = future.result()
                for line in result.stdout.splitlines():
                    if line.startswith('{'):
                        print(line, flush=True)
                exit_code = docker('inspect', '--format', '{{.State.ExitCode}}', client).stdout.strip()
                if (result.returncode or exit_code != '0') and not failed:
                    failed = True
                    for other in clients:
                        if other != client:
                            docker('stop', '--time', '2', other, check=False)
            if failed:
                raise RuntimeError('Concurrent diagnostic failed; retained private artifacts contain the original CLI output')
            invalid = []
            for chunk in [1, 2, 3]:
                validation = docker('run', '--rm', '--network', 'none', '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--pids-limit', '128', '--memory', '768m', '--cpus', '1', '--tmpfs', '/tmp:size=256m', '--mount', f'type=bind,source={root / f"chunk-{chunk}"},target=/diagnostic,readonly', '-e', 'OCR_DIAGNOSTIC_EXPORT_STAGE=/diagnostic', args.test_image, 'review::usage_tests::validate_retained_real_chunk_report', '--exact', '--ignored', '--nocapture', '--test-threads=1', check=False)
                for line in validation.stdout.splitlines():
                    if 'REAL_REPORT_VALIDATION ' in line:
                        print(line.split('REAL_REPORT_VALIDATION ', 1)[1], flush=True)
                if validation.returncode:
                    invalid.append(chunk)
            if invalid:
                raise RuntimeError(f'CLI execution completed, but retained reports failed validation for chunks {invalid}')
    finally:
        try:
            for collector in collectors:
                try:
                    collector.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    collector.kill()
                    collector.wait(timeout=5)
                finally:
                    collector.stderr.close()
        finally:
            for stream in streams:
                stream.close()
            if prefix + '-proxy' in containers:
                logs = docker('logs', prefix + '-proxy', check=False)
                (root / 'proxy.log').write_text(logs.stdout + logs.stderr)
                (root / 'proxy.log').chmod(0o600)
            for chunk in [1, 2, 3]:
                runner = f'{prefix}-runner-{chunk}'
                if runner in containers:
                    logs = docker('logs', runner, check=False)
                    path = root / f'chunk-{chunk}' / 'supervisor.log'
                    path.write_text(logs.stdout + logs.stderr)
                    path.chmod(0o600)
            for name in reversed(containers):
                docker('stop', '--time', '2', name, check=False)
                docker('rm', '-v', name, check=False)
            for name in reversed(networks):
                docker('network', 'rm', name, check=False)
            for name in reversed(volumes):
                docker('volume', 'rm', name, check=False)
            print(json.dumps({'retainedArtifacts': str(root), 'operationalStackChanged': False}), flush=True)


if __name__ == '__main__':
    main()
