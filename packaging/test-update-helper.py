#!/usr/bin/env python3
"""Exercise a packaged updater against a disposable native installation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time

parser = argparse.ArgumentParser()
parser.add_argument('--helper', type=Path, required=True)
parser.add_argument('--artifact', type=Path, required=True)
parser.add_argument('--target', type=Path, required=True)
parser.add_argument('--expected-binary', type=Path, required=True)
args = parser.parse_args()
relative_binary = Path('bin/typsmthng.exe') if sys.platform == 'win32' else Path('Contents/MacOS/typsmthng')
binary = args.target.resolve() / relative_binary
expected = hashlib.sha256(args.expected_binary.read_bytes()).hexdigest()
# Prove replacement, not just a no-op successful helper exit.
binary.write_bytes(b'old installation placeholder\n')
with tempfile.TemporaryDirectory(prefix='typsmthng-helper-test-') as directory:
    directory = Path(directory)
    helper = directory / args.helper.name
    shutil.copy2(args.helper, helper)
    if sys.platform == 'win32':
        for dll in args.helper.parent.glob('*.dll'):
            shutil.copy2(dll, directory / dll.name)
    artifact = directory / args.artifact.name
    shutil.copy2(args.artifact, artifact)
    job = {'artifact': str(artifact), 'target': str(args.target.resolve()),
           'sha256': hashlib.sha256(artifact.read_bytes()).hexdigest()}
    with (directory / 'helper.log').open('w+') as log:
        process = subprocess.Popen([str(helper)], stdin=subprocess.PIPE,
                                   stdout=subprocess.PIPE, stderr=log, text=True)
        try:
            process.stdin.write(json.dumps(job) + '\n')
            process.stdin.flush()
            reply = queue.Queue()
            threading.Thread(target=lambda: reply.put(process.stdout.readline()), daemon=True).start()
            assert reply.get(timeout=30).strip() == 'ready', 'helper did not acknowledge readiness'
            process.stdin.write('install\n')
            process.stdin.flush()
            time.sleep(0.25)
            assert process.poll() is None, 'helper did not wait for parent exit'
            assert binary.read_bytes() == b'old installation placeholder\n', 'installed before exit'
            process.stdin.close()
            assert process.wait(timeout=180) == 0, 'helper installation failed'
            assert hashlib.sha256(binary.read_bytes()).hexdigest() == expected, 'wrong installed binary'
            # Verify relaunch, then stop only the disposable test installation.
            if sys.platform == 'win32':
                env = dict(os.environ, TYPSMTHNG_TEST_BINARY=str(binary))
                subprocess.run(['powershell', '-NoProfile', '-Command', '''
$ErrorActionPreference = 'Stop'
for ($i = 0; $i -lt 60; $i++) {
  $apps = @(Get-CimInstance Win32_Process -Filter "Name='typsmthng.exe'" | Where-Object { $_.ExecutablePath -eq $env:TYPSMTHNG_TEST_BINARY })
  if ($apps.Count -gt 0) { $apps | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }; exit 0 }
  Start-Sleep -Milliseconds 500
}
throw 'Updated application did not relaunch'
'''], env=env, check=True, timeout=45)
            else:
                launched = False
                for _ in range(60):
                    processes = subprocess.check_output(['ps', '-axo', 'pid=,comm='], text=True)
                    for line in processes.splitlines():
                        fields = line.strip().split(None, 1)
                        if len(fields) == 2 and fields[1] == str(binary):
                            os.kill(int(fields[0]), 15)
                            launched = True
                    if launched:
                        break
                    time.sleep(0.5)
                assert launched, 'Updated application did not relaunch'
            time.sleep(1)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
            log.seek(0)
            print(log.read())
print('Packaged updater waited for exit, replaced the app, and relaunched successfully.')
