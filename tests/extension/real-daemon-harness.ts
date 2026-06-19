import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { bridgeUrl, dappOrigin, repoRoot } from './extension-fixtures';

export const realDaemonAccount = '0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266';
export const realDaemonChainId = '0x7a69';

export type RealDaemonScenario = 'unlocked' | 'locked' | 'missing-socket' | 'wrong-chain';

type ChildHandle = {
  child: ChildProcessWithoutNullStreams;
  logs: string[];
  label: string;
};

type RealDaemonRuntime = {
  qaDir: string;
  socketPath: string;
  stop: () => Promise<void>;
};

export async function startRealDaemonBridge(
  scenario: RealDaemonScenario,
  _suffix: string,
): Promise<RealDaemonRuntime> {
  if (await bridgeListening()) {
    throw new Error('Deckard browser bridge is already listening on 127.0.0.1:8765; stop it first');
  }

  const qaDir = fs.mkdtempSync(path.join(os.tmpdir(), `dqa-${scenarioPrefix(scenario)}-`));

  const socketPath = path.join(qaDir, 'signerd.sock');
  const children: ChildHandle[] = [];

  try {
    if (scenario !== 'missing-socket') {
      await runToCompletion(
        'cargo',
        [
          'run',
          '--quiet',
          '--locked',
          '-p',
          'deckard-core',
          '--no-default-features',
          '--example',
          'qa-vault',
        ],
        {
          DECKARD_CONFIG_DIR: qaDir,
        },
      );

      const signerd = spawnLogged(
        'signerd',
        'cargo',
        [
          'run',
          '--quiet',
          '--locked',
          '-p',
          'deckard-signerd',
          '--no-default-features',
          '--bin',
          'deckard-signerd',
        ],
        {
          DECKARD_CHAIN_ID: scenario === 'wrong-chain' ? '1' : '31337',
          DECKARD_CONFIG_DIR: qaDir,
          DECKARD_RPC_URL: 'http://127.0.0.1:1',
          DECKARD_SOCKET_PATH: socketPath,
        },
      );
      children.push(signerd);
      await waitForSocket(signerd, socketPath);

      if (scenario !== 'locked') {
        await runToCompletion(
          'cargo',
          [
            'run',
            '--quiet',
            '--locked',
            '-p',
            'deckard-signerd',
            '--no-default-features',
            '--example',
            'qa-unlock',
          ],
          {
            DECKARD_SOCKET_PATH: socketPath,
          },
        );
      }
    }

    const bridge = spawnLogged(
      'bridge',
      'cargo',
      [
        'run',
        '--quiet',
        '--locked',
        '-p',
        'deckard-browser-bridge',
        '--no-default-features',
        '--',
        '--bind',
        '127.0.0.1:8765',
      ],
      {
        DECKARD_CHAIN_ID: '31337',
        DECKARD_CONFIG_DIR: qaDir,
        DECKARD_RPC_URL: 'http://127.0.0.1:1',
        DECKARD_SOCKET_PATH: socketPath,
      },
    );
    children.push(bridge);
    await waitForBridge(bridge);

    return {
      qaDir,
      socketPath,
      stop: async () => {
        await stopChildren(children);
        fs.rmSync(qaDir, { recursive: true, force: true });
      },
    };
  } catch (error) {
    await stopChildren(children);
    fs.rmSync(qaDir, { recursive: true, force: true });
    throw error;
  }
}

export async function requestBridge(method: string, id = 1) {
  const response = await fetch(bridgeUrl, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      'x-deckard-origin': dappOrigin,
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id,
      method,
      params: [],
    }),
    signal: AbortSignal.timeout(5_000),
  });
  if (!response.ok) {
    throw new Error(`bridge HTTP ${response.status}: ${await response.text()}`);
  }
  return response.json();
}

function spawnLogged(
  label: string,
  command: string,
  args: string[],
  env: Record<string, string>,
): ChildHandle {
  const child = spawn(command, args, {
    cwd: repoRoot,
    env: {
      ...process.env,
      ...env,
    },
  });
  const logs: string[] = [];
  collectLogs(child, logs);
  return { child, logs, label };
}

async function runToCompletion(command: string, args: string[], env: Record<string, string>) {
  const child = spawn(command, args, {
    cwd: repoRoot,
    env: {
      ...process.env,
      ...env,
    },
  });
  const logs: string[] = [];
  collectLogs(child, logs);
  const code = await exitCode(child);
  if (code !== 0) {
    throw new Error(`${command} ${args.join(' ')} failed with ${code}:\n${logs.join('')}`);
  }
}

function collectLogs(child: ChildProcessWithoutNullStreams, logs: string[]) {
  const collect = (chunk: Buffer) => {
    logs.push(chunk.toString());
    if (logs.length > 60) {
      logs.shift();
    }
  };
  child.stdout.on('data', collect);
  child.stderr.on('data', collect);
}

async function waitForSocket(handle: ChildHandle, socketPath: string) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (handle.child.exitCode !== null) {
      throw new Error(`${handle.label} exited early:\n${handle.logs.join('')}`);
    }
    if (fs.existsSync(socketPath)) {
      return;
    }
    await delay(250);
  }
  throw new Error(`${handle.label} did not create ${socketPath}:\n${handle.logs.join('')}`);
}

async function waitForBridge(handle: ChildHandle) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (handle.child.exitCode !== null) {
      throw new Error(`${handle.label} exited early:\n${handle.logs.join('')}`);
    }
    if (await bridgeReady(realDaemonChainId)) {
      return;
    }
    await delay(250);
  }
  throw new Error(`${handle.label} did not become ready:\n${handle.logs.join('')}`);
}

async function bridgeReady(expectedChainId: string) {
  try {
    const payload = await requestBridge('eth_chainId');
    return payload.result === expectedChainId;
  } catch {
    return false;
  }
}

async function bridgeListening() {
  try {
    await requestBridge('eth_chainId');
    return true;
  } catch {
    return false;
  }
}

async function stopChildren(children: ChildHandle[]) {
  for (const handle of [...children].reverse()) {
    await stopChild(handle.child);
  }
}

async function stopChild(child: ChildProcessWithoutNullStreams) {
  if (child.exitCode !== null) {
    return;
  }
  child.kill('SIGTERM');
  await new Promise<void>((resolve) => {
    const timeout = setTimeout(() => {
      child.kill('SIGKILL');
      resolve();
    }, 2_000);
    child.once('exit', () => {
      clearTimeout(timeout);
      resolve();
    });
  });
}

async function exitCode(child: ChildProcessWithoutNullStreams): Promise<number | null> {
  return new Promise((resolve) => {
    child.once('exit', (code) => resolve(code));
  });
}

function delay(ms: number) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function scenarioPrefix(scenario: RealDaemonScenario) {
  return scenario
    .split('-')
    .map((part) => part[0])
    .join('');
}
