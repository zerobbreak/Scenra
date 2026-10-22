import { useState } from "react";
import Button from "./Button";
import { CapabilityProfile, launchTarget, stopTarget, targetProfile } from "./target";

export default function TargetPanel() {
  const [profile, setProfile] = useState<CapabilityProfile | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<CapabilityProfile | null | void>) {
    setBusy(true);
    setError(null);
    try {
      setProfile((await action()) ?? null);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const cdp = profile?.capabilities.cdp;

  return (
    <section className="space-y-4">
      <header>
        <p className="text-xs uppercase tracking-widest text-indigo-400">Phase 0 · Step 2</p>
        <h2 className="text-lg font-semibold">Attach to the demo target over CDP</h2>
        <p className="mt-1 text-sm text-zinc-400">
          Launches Lumina with a WebView2 debug port set for that process only, then builds its
          capability profile. Build it first: <code>cargo build -p demo-target</code>
        </p>
      </header>

      <div className="flex flex-wrap gap-2">
        <Button disabled={busy} onClick={() => run(launchTarget)}>
          {busy ? "Working…" : "Launch demo target"}
        </Button>
        <Button disabled={busy || !profile} onClick={() => run(targetProfile)}>
          Re-detect
        </Button>
        <Button disabled={busy || !profile} onClick={() => run(stopTarget)}>
          Stop
        </Button>
      </div>

      {profile && cdp && (
        <div className="space-y-3 rounded-lg bg-zinc-900 p-4 text-sm">
          <p className={cdp.available ? "text-emerald-400" : "text-amber-400"}>
            {cdp.available ? `Fully supported · cdp: ${cdp.flavor}` : "Limited: no CDP"}
          </p>
          <dl className="grid grid-cols-[10rem_1fr] gap-y-1">
            <dt className="text-zinc-400">Application</dt>
            <dd>{profile.application}</dd>
            <dt className="text-zinc-400">Process</dt>
            <dd>
              {profile.process.exe} (pid {profile.process.pid})
            </dd>
            <dt className="text-zinc-400">Endpoint</dt>
            <dd>
              {cdp.endpoint ?? "—"} {cdp.endpointProcess && `· ${cdp.endpointProcess}`}
            </dd>
            <dt className="text-zinc-400">Browser</dt>
            <dd>{cdp.browser ?? "—"}</dd>
            <dt className="text-zinc-400">Owned by target</dt>
            <dd>{cdp.ownedByTarget ? "yes" : "no"}</dd>
            <dt className="text-zinc-400">Loopback only</dt>
            <dd>{cdp.loopbackOnly ? "yes" : "no"}</dd>
            <dt className="text-zinc-400">Page</dt>
            <dd className="break-all">{cdp.pageUrl ?? "—"}</dd>
            <dt className="text-zinc-400">data-testid elements</dt>
            <dd>{cdp.testIds ?? "—"}</dd>
          </dl>
          {profile.warnings.length > 0 && (
            <ul className="list-disc pl-5 text-amber-300">
              {profile.warnings.map((w) => (
                <li key={w}>{w}</li>
              ))}
            </ul>
          )}
          <details>
            <summary className="cursor-pointer text-zinc-400">Raw profile</summary>
            <pre className="mt-2 overflow-x-auto text-xs">{JSON.stringify(profile, null, 2)}</pre>
          </details>
        </div>
      )}

      {error && <p className="rounded-lg bg-red-950 p-3 text-sm text-red-200">{error}</p>}
    </section>
  );
}
