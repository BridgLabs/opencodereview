import argparse
import json
import pathlib
import select
import subprocess
import time
import uuid


COLLECTOR = r'''
use strict; use warnings; use Fcntl qw(O_RDONLY O_NOFOLLOW); use JSON::PP; use Encode qw(decode FB_DEFAULT);
my $root = $ARGV[0] // '/scratch';
my $deadline = time + 1000;
my ($stdout, $stderr);
print STDERR "CLI_CAPTURE_READY\n";
while (time < $deadline) {
    if (-f "$root/stderr" && sysopen($stderr, "$root/stderr", O_RDONLY | O_NOFOLLOW)) {
        sysopen($stdout, "$root/transcript.jsonl", O_RDONLY | O_NOFOLLOW) or die "Cannot capture CLI stdout";
        print STDERR "CLI_CAPTURE_OPENED\n";
        last;
    }
    select undef, undef, undef, 0.01;
}
die "CLI execution output was not created" unless $stdout && $stderr;
my %logs;
while (-e "$root/stderr" && time < $deadline) {
    for my $path (glob("$root/session/.claude/debug/*.txt")) {
        next if exists $logs{$path} || scalar(keys %logs) >= 16;
        if (sysopen(my $log, $path, O_RDONLY | O_NOFOLLOW)) { $logs{$path} = $log; }
    }
    select undef, undef, undef, 0.05;
}
die "CLI output cleanup deadline exceeded" if -e "$root/stderr";
sub captured {
    my ($file) = @_;
    seek($file, 0, 0) or die "Cannot rewind CLI output";
    my $text = '';
    while (1) {
        my $count = read($file, my $part, 65536);
        die "Cannot read CLI output" unless defined $count;
        last unless $count;
        $text .= $part;
        die "CLI output exceeds diagnostic limit" if length($text) > 8000000;
    }
    return decode('UTF-8', $text, FB_DEFAULT);
}
print encode_json({stdout => captured($stdout), stderr => captured($stderr), logs => [map { my $name = $_; $name =~ s{.*/}{}; {name => $name, content => captured($logs{$_})} } sort keys %logs]});
'''


CLIENT = r'''
import base64, hashlib, json, os, pathlib, socket, struct
os.umask(0o077)
root = pathlib.Path('/diagnostic')
packet = json.loads((root / 'stage.json').read_text())
raw = (root / 'snapshot.archive').read_bytes()
assert raw[:8] == b'OCRARCH1'
length = struct.unpack('<I', raw[8:12])[0]
manifest = raw[12:12+length]
assert hashlib.sha256(manifest).hexdigest() == packet['descriptor']['id']
metadata = json.loads(manifest)
data = raw[12+length:]
request = packet['request']

def connect():
    stream = socket.socket(socket.AF_UNIX)
    stream.settimeout(950)
    stream.connect(os.environ['OCR_DIAGNOSTIC_RUNNER_SOCKET'])
    return stream

def send(stream, value):
    encoded = json.dumps(value, separators=(',', ':')).encode()
    assert len(encoded) <= 48 * 1024 * 1024
    stream.sendall(struct.pack('!I', len(encoded)) + encoded)

def exact(stream, count):
    result = bytearray()
    while len(result) < count:
        part = stream.recv(count-len(result))
        if not part:
            raise RuntimeError('Runner IPC closed')
        result.extend(part)
    return bytes(result)

def receive(stream):
    count = struct.unpack('!I', exact(stream, 4))[0]
    assert 0 < count <= 48 * 1024 * 1024
    return json.loads(exact(stream, count))

def install(stream):
    send(stream, {'operation':'Install','engine_version':packet['engineVersion'],'descriptor':packet['descriptor'],'manifest':manifest.decode()})
    assert receive(stream)['event'] == 'Ack'
    for blob in metadata['blobs']:
        content = data[blob['offset']:blob['offset']+blob['bytes']]
        assert hashlib.sha256(content).hexdigest() == blob['hash']
        send(stream, {'operation':'Blob','hash':blob['hash'],'data':base64.b64encode(content).decode()})
        assert receive(stream)['event'] == 'Ack'
    send(stream, {'operation':'Seal'})
    assert receive(stream)['event'] == 'Ack'

with connect() as stream:
    send(stream, {'operation':'Catalog'})
    catalogue = receive(stream)
    assert catalogue['event'] == 'Catalog', 'Real runner catalogue unavailable'
    assert request['model'] in catalogue['models'], 'Recorded model unavailable'
    request['version'] = catalogue['version']
print(json.dumps({'phase':'catalogue','model':request['model'],'version':request['version']}), flush=True)
with connect() as stream:
    install(stream)
    send(stream, dict(request, preflight=True))
    result = receive(stream)
    assert result['event'] == 'Ready', 'Real runner preflight failed'
print(json.dumps({'phase':'preflight','modelCalls':0}), flush=True)
with connect() as stream:
    install(stream)
    with (root / 'execution-once.json').open('x') as marker:
        json.dump({'runId':packet['runId'],'chunk':packet['chunk'],'model':request['model'],'maximumExecutions':1}, marker)
    send(stream, dict(request, preflight=False))
    started = False
    while True:
        event = receive(stream)
        if event['event'] == 'Started':
            assert not started
            started = True
            print(json.dumps({'phase':'running','runId':packet['runId'],'chunk':packet['chunk'],'model':request['model']}), flush=True)
        elif event['event'] in ['Finished', 'Error']:
            (root / 'runner-result.json').write_text(json.dumps(event))
            for name, key in [('report.json','report'),('finding_decisions.json','decisions'),('transcript.json','transcript')]:
                if event.get(key) is not None:
                    (root / name).write_text(event[key])
            print(json.dumps({'phase':'finished','started':started,'runnerError':event.get('error') is not None,'reportSaved':event.get('report') is not None}), flush=True)
            if event.get('error') is not None:
                raise RuntimeError('Runner failed; retained diagnostic artifacts contain details')
            break
        else:
            assert event['event'] == 'Pulse'
'''


def docker(*args, check=True, timeout=120):
    result = subprocess.run(['docker', *args], capture_output=True, text=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f'docker {args[0]} failed: {result.stderr.strip()}')
    return result


def execution_limit(single_attempt, repair_final, maximum):
    if repair_final:
        if maximum not in (None, 1):
            raise ValueError('Final repair authorizes exactly one execution')
        return 1
    if single_attempt:
        if maximum is None or maximum < 1:
            raise ValueError('--single-attempt-per-stage requires a positive --max-executions covering chunks and all Final batches')
        return maximum
    if maximum is not None:
        raise ValueError('--max-executions requires --single-attempt-per-stage or --repair-final-only')
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--directory', required=True)
    parser.add_argument('--image', required=True)
    authorization = parser.add_mutually_exclusive_group(required=True)
    authorization.add_argument('--authorize-one-model-execution', action='store_true')
    authorization.add_argument('--authorize-complete-review', action='store_true')
    parser.add_argument('--test-image')
    parser.add_argument('--parallel-runners', type=int, choices=[1, 2, 3], default=1)
    parser.add_argument('--single-attempt-per-stage', action='store_true')
    parser.add_argument('--max-executions', type=int, help='Explicit total attempt ceiling for chunks and dynamically partitioned Final batches')
    parser.add_argument('--final-only', action='store_true')
    parser.add_argument('--repair-final-only', action='store_true')
    parser.add_argument('--capture-cli-output', action='store_true')
    reuse = parser.add_mutually_exclusive_group()
    reuse.add_argument('--reuse-chunk')
    reuse.add_argument('--reuse-review')
    args = parser.parse_args()
    try:
        maximum = execution_limit(args.single_attempt_per_stage, args.repair_final_only, args.max_executions)
    except ValueError as error:
        parser.error(str(error))
    if (args.parallel_runners != 1 or args.single_attempt_per_stage) and not args.authorize_complete_review:
        parser.error('Parallel runners and a stage execution budget require complete-review authorization')
    if args.capture_cli_output and not args.authorize_one_model_execution:
        parser.error('CLI output capture requires single-stage authorization')
    if args.authorize_complete_review and not args.test_image:
        parser.error('Complete review requires the matching diagnostic test image')
    if args.repair_final_only and (not args.authorize_complete_review or not args.reuse_review or args.parallel_runners != 1 or args.final_only):
        parser.error('Final repair requires retained chunks, complete-review authorization and exactly one runner')
    if args.final_only and (not args.authorize_complete_review or not args.reuse_review):
        parser.error('Final-only validation requires complete-review authorization and retained chunks')
    root = pathlib.Path(args.directory).resolve()
    if root.is_symlink() or root.stat().st_mode & 0o077:
        raise RuntimeError('A private retained diagnostic directory is required')
    packet = json.loads((root / ('coordinator.json' if args.authorize_complete_review else 'stage.json')).read_text())
    marker = 'full-execution-once.json' if args.authorize_complete_review else 'execution-once.json'
    if (root / marker).exists():
        raise RuntimeError('This diagnostic already consumed its execution authorization')
    engine = docker('run', '--rm', '--network', 'none', '--read-only', args.image, 'engine-version').stdout.strip()
    if engine != packet['engineVersion']:
        raise RuntimeError('Diagnostic executor and exported stage differ')
    prefix = 'ocr-real-diagnostic-' + uuid.uuid4().hex[:12]
    containers, networks, volumes = [], [], []
    collector = captured_output = None
    try:
        outbound, internal, ipc = prefix + '-outbound', prefix + '-internal', prefix + '-ipc'
        for name, options in [(outbound, []), (internal, ['--internal'])]:
            docker('network', 'create', *options, name)
            networks.append(name)
        docker('volume', 'create', ipc)
        volumes.append(ipc)
        common = ['--init', '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--pids-limit', '128', '--cpus', '1', '--memory', '2g', '--memory-swap', '2g', '--tmpfs', '/data:size=1m,mode=0700', '--tmpfs', '/tmp:size=64m,mode=1777']
        proxy = prefix + '-proxy'
        docker('create', '--name', proxy, *common, '--network', outbound, '--user', '10001:10001', '--cpus', '0.25', '--memory', '128m', '--memory-swap', '128m', args.image, 'egress')
        containers.append(proxy)
        docker('network', 'connect', '--alias', 'diagnostic-egress', internal, proxy)
        docker('start', proxy)
        runner = prefix + '-runner'
        docker('create', '--name', runner, '--hostname', runner, *common, '--network', internal, '--cap-add', 'SETUID', '--cap-add', 'SETGID', '--cap-add', 'CHOWN', '--cap-add', 'DAC_OVERRIDE', '--cap-add', 'KILL', '--mount', f'type=volume,source={ipc},target=/ipc', '--mount', 'type=volume,source=opencodereview_review-claude-auth,target=/claude-auth,readonly', '-e', 'OCR_RUNNER_CLAUDE_TOKEN=/claude-auth/app-managed/claude/oauth-token', '-e', 'OCR_RUNNER_PROXY=http://diagnostic-egress:8790', '--tmpfs', '/work:size=16m,mode=0755', '--tmpfs', '/evidence:size=512m,mode=0755', '--tmpfs', '/output:size=48m,mode=0777', '--tmpfs', '/scratch:size=128m,mode=0755', args.image, 'runner')
        containers.append(runner)
        docker('start', runner)
        for _ in range(30):
            if docker('exec', runner, 'test', '-S', f'/ipc/{runner}.sock', check=False).returncode == 0:
                break
            time.sleep(0.2)
        else:
            raise RuntimeError('Diagnostic runner did not become ready')
        runner_sockets = [f'/ipc/{runner}.sock']
        extra_mounts = []
        for index in range(2, args.parallel_runners + 1):
            extra_network, extra_ipc = f'{prefix}-internal-{index}', f'{prefix}-ipc-{index}'
            docker('network', 'create', '--internal', extra_network)
            networks.append(extra_network)
            docker('network', 'connect', '--alias', 'diagnostic-egress', extra_network, proxy)
            docker('volume', 'create', extra_ipc)
            volumes.append(extra_ipc)
            extra_runner = f'{prefix}-runner-{index}'
            docker('create', '--name', extra_runner, '--hostname', extra_runner, *common, '--network', extra_network, '--cap-add', 'SETUID', '--cap-add', 'SETGID', '--cap-add', 'CHOWN', '--cap-add', 'DAC_OVERRIDE', '--cap-add', 'KILL', '--mount', f'type=volume,source={extra_ipc},target=/ipc', '--mount', 'type=volume,source=opencodereview_review-claude-auth,target=/claude-auth,readonly', '-e', 'OCR_RUNNER_CLAUDE_TOKEN=/claude-auth/app-managed/claude/oauth-token', '-e', 'OCR_RUNNER_PROXY=http://diagnostic-egress:8790', '--tmpfs', '/work:size=16m,mode=0755', '--tmpfs', '/evidence:size=512m,mode=0755', '--tmpfs', '/output:size=48m,mode=0777', '--tmpfs', '/scratch:size=128m,mode=0755', args.image, 'runner')
            containers.append(extra_runner)
            docker('start', extra_runner)
            for _ in range(30):
                if docker('exec', extra_runner, 'test', '-S', f'/ipc/{extra_runner}.sock', check=False).returncode == 0:
                    break
                time.sleep(0.2)
            else:
                raise RuntimeError('Additional diagnostic runner did not become ready')
            extra_mounts.extend(['--mount', f'type=volume,source={extra_ipc},target=/ipc-{index},readonly'])
            runner_sockets.append(f'/ipc-{index}/{extra_runner}.sock')
        if args.capture_cli_output:
            captured_output = (root / 'cli-console.json').open('x')
            (root / 'cli-console.json').chmod(0o600)
            collector = subprocess.Popen(['docker', 'exec', runner, 'perl', '-e', COLLECTOR], stdout=captured_output, stderr=subprocess.PIPE)
            if not select.select([collector.stderr], [], [], 5)[0] or collector.stderr.readline() != b'CLI_CAPTURE_READY\n':
                raise RuntimeError('CLI diagnostic collector failed before execution')
        client = prefix + '-client'
        client_args = ['--network', 'none', '--read-only', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges:true', '--memory', '1g', '--cpus', '1', '--tmpfs', '/tmp:size=128m', '--mount', f'type=volume,source={ipc},target=/ipc,readonly', '--mount', f'type=bind,source={root},target=/diagnostic']
        if args.authorize_complete_review:
            client_args.extend(extra_mounts)
            client_args.extend(['-e', 'OCR_DIAGNOSTIC_RUNNER_SOCKETS=' + ','.join(runner_sockets)])
            if maximum is not None:
                client_args.extend(['-e', 'OCR_DIAGNOSTIC_MAX_EXECUTIONS=' + str(maximum)])
            if args.repair_final_only:
                client_args.extend(['-e', 'OCR_DIAGNOSTIC_REPAIR_FINAL_ONLY=true'])
            if args.final_only:
                client_args.extend(['-e', 'OCR_DIAGNOSTIC_FINAL_ONLY=true'])
            if args.reuse_chunk or args.reuse_review:
                reuse = pathlib.Path(args.reuse_chunk or args.reuse_review).resolve()
                variable = 'OCR_DIAGNOSTIC_REUSE_REVIEW' if args.reuse_review else 'OCR_DIAGNOSTIC_REUSE_CHUNK'
                client_args.extend(['--mount', f'type=bind,source={reuse},target=/reuse,readonly', '-e', variable + '=/reuse'])
            client_args.extend(['-e', 'OCR_DIAGNOSTIC_EXPORT_STAGE=/diagnostic', '-e', 'OCR_DIAGNOSTIC_RUNNER_SOCKET=' + runner_sockets[0], '-e', 'OCR_DIAGNOSTIC_FULL_REVIEW_AUTHORIZED=' + str(packet['runId']), args.test_image, 'review::usage_tests::validate_complete_real_review', '--exact', '--ignored', '--nocapture', '--test-threads=1'])
        else:
            client_args.extend(['-e', 'OCR_DIAGNOSTIC_RUNNER_SOCKET=' + runner_sockets[0], 'python:3.12-slim', 'python3', '-u', '-c', CLIENT])
        docker('create', '--name', client, *client_args)
        containers.append(client)
        subprocess.run(['docker', 'start', '-a', client], check=True, timeout=86400 if args.authorize_complete_review else 1100)
        if docker('inspect', '--format', '{{.State.ExitCode}}', client).stdout.strip() != '0':
            raise RuntimeError('Diagnostic client failed; inspect retained artifacts before any retry')
    finally:
        try:
            if collector is not None:
                try:
                    collector.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    collector.kill()
                    collector.wait(timeout=5)
                finally:
                    collector.stderr.close()
        finally:
            if captured_output is not None:
                captured_output.close()
            for name in reversed(containers):
                docker('stop', '--time', '5', name, check=False)
                docker('rm', '-v', name, check=False)
            for name in reversed(networks):
                docker('network', 'rm', name, check=False)
            for name in reversed(volumes):
                docker('volume', 'rm', name, check=False)
    print(json.dumps({'retainedArtifacts':str(root),'operationalStackChanged':False}), flush=True)


if __name__ == '__main__':
    main()
