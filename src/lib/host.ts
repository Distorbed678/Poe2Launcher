import type { HostSettings, RunState } from './types';

/** What the setup wizard edits (the admin password never leaves this object and the page). */
export interface HostForm {
  server_name: string;
  cluster_id: string;
  login_port: number;
  instance_port: number;
  web_port: number;
  public_host: string;
  web_public: boolean;
  open_registration: boolean;
  admin_name: string;
  admin_password: string;
  admin_password2: string;
}

export function formFromSettings(s: HostSettings, lanIp: string | null): HostForm {
  return {
    server_name: s.server_name,
    cluster_id: s.cluster_id,
    login_port: s.login_port,
    instance_port: s.instance_port,
    web_port: s.web_port,
    // A fresh setup still has the loopback default: offer the LAN address instead.
    public_host: !s.configured && s.public_host === '127.0.0.1' && lanIp ? lanIp : s.public_host,
    web_public: s.web_public,
    open_registration: s.open_registration,
    admin_name: s.admin_name,
    admin_password: '',
    admin_password2: '',
  };
}

export function settingsFromForm(base: HostSettings, f: HostForm): HostSettings {
  return {
    ...base,
    server_name: f.server_name.trim(),
    cluster_id: f.cluster_id.trim(),
    login_port: f.login_port,
    instance_port: f.instance_port,
    web_port: f.web_port,
    public_host: f.public_host.trim(),
    web_public: f.web_public,
    open_registration: f.open_registration,
    admin_name: f.admin_name.trim(),
  };
}

const ACCOUNT_NAME = /^[A-Za-z0-9_.@+-]{3,64}$/;
const CLUSTER_ID = /^[a-z0-9_-]{1,32}$/;

/** An IP address or a plain host name (mirrors `valid_public_host`). */
export function validPublicHost(host: string): boolean {
  const h = host.trim();
  if (!h || h.length > 253) return false;
  if (/^[0-9a-fA-F:]+$/.test(h) && h.includes(':')) return true; // IPv6
  return h.split('.').every((l) => l.length > 0 && l.length <= 63 && /^[A-Za-z0-9-]+$/.test(l) && !l.startsWith('-') && !l.endsWith('-'));
}

/** Problems with the form (mirrors `HostSettings::validate`; the backend checks again). */
export function validateHostForm(f: HostForm, needPassword: boolean): string[] {
  const out: string[] = [];
  const name = f.server_name.trim();
  if (!name || name.length > 64) out.push('The server name is 1-64 characters.');
  if (!CLUSTER_ID.test(f.cluster_id)) out.push('The cluster id is 1-32 characters: lower-case letters, digits, - and _.');
  const ports: [string, number][] = [
    ['login', f.login_port],
    ['instance', f.instance_port],
    ['web', f.web_port],
  ];
  for (const [what, p] of ports) {
    if (!Number.isInteger(p) || p < 1024 || p > 65535) out.push(`The ${what} port is a number from 1024 to 65535.`);
  }
  if (new Set(ports.map(([, p]) => p)).size !== 3) out.push('The login, instance and web ports must be three different numbers.');
  if (!validPublicHost(f.public_host)) out.push('The public host is an IP address or a host name, without http:// or a port.');
  if (needPassword || f.admin_name.trim()) {
    if (!ACCOUNT_NAME.test(f.admin_name.trim())) out.push('The admin account name is 3-64 characters: letters, digits and _ . @ + -');
  }
  if (needPassword || f.admin_password) {
    if (f.admin_password.length < 8) out.push('The admin password needs at least 8 characters.');
    else if (f.admin_password.length > 256) out.push('The admin password is at most 256 characters.');
    else if (f.admin_password !== f.admin_password2) out.push('The two passwords differ.');
  }
  return out;
}

export interface ForwardRule {
  port: number;
  proto: 'TCP' | 'UDP' | 'TCP + UDP';
  what: string;
}

/** The ports a router has to forward for players outside the local network. */
export function portsToForward(f: Pick<HostForm, 'login_port' | 'instance_port' | 'web_port' | 'web_public'>): ForwardRule[] {
  const rules: ForwardRule[] = [
    { port: f.login_port, proto: 'TCP', what: 'login server' },
    { port: f.instance_port, proto: 'TCP + UDP', what: 'game instances (TCP) and the client hook side channel (UDP)' },
  ];
  if (f.web_public) rules.push({ port: f.web_port, proto: 'TCP', what: 'web API and web admin (plain http: see the hosting guide for TLS)' });
  return rules;
}

/** The address players enter in their launcher for this server. */
export function playerAddress(f: Pick<HostForm, 'public_host' | 'web_port'>): string {
  const h = f.public_host.trim();
  return `${h.includes(':') ? `[${h}]` : h}:${f.web_port}`;
}

export function runLabel(run: RunState, ready: boolean): string {
  switch (run.state) {
    case 'running':
      return ready ? `Running (pid ${run.pid})` : `Starting (pid ${run.pid}), reading the game data...`;
    case 'stopping':
      return 'Stopping...';
    case 'exited':
      return run.code === 0 || run.code === null ? 'Ended' : `Stopped unexpectedly (exit code ${run.code})`;
    default:
      return 'Stopped';
  }
}

export function isUp(run: RunState): boolean {
  return run.state === 'running' || run.state === 'stopping';
}

export const MAX_HOST_LOG = 2000;

/** Appends a line, keeping the newest `max`. */
export function pushLog(lines: string[], line: string, max = MAX_HOST_LOG): string[] {
  const next = lines.length >= max ? lines.slice(lines.length - max + 1) : lines.slice();
  next.push(line);
  return next;
}

/** A class for a log line (`error` / `warn` for the server's own levels). */
export function logLevel(line: string): 'error' | 'warn' | 'launcher' | 'plain' {
  if (line.startsWith('[launcher]')) return 'launcher';
  if (/\bERROR\b/.test(line)) return 'error';
  if (/\bWARN\b/.test(line)) return 'warn';
  return 'plain';
}
