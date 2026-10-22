import { useEffect, useState } from "react";
import Button from "./Button";
import { openTeleprompter } from "./capture";
import { AiStatus, Mode, Scenario, aiStatus, currentScript, draftScript, startSession, words } from "./script";
import { launchChrome } from "./target";

const DEFAULT_URL = "https://lumina-web-production-e6ce.up.railway.app/";
const DEFAULT_BRIEF = "A friendly 60-second tour of Lumina for students: what it does in a lecture, then a look at revision flashcards.";

export default function ScriptPanel() {
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [url, setUrl] = useState(DEFAULT_URL);
  const [brief, setBrief] = useState(DEFAULT_BRIEF);
  const [script, setScript] = useState<Scenario | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    aiStatus().then(setStatus);
    currentScript().then(setScript);
  }, []);

  async function run(label: string, action: () => Promise<void>) {
    setBusy(label);
    setError(null);
    setNote(null);
    try {
      await action();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  // Rehearse reads the whole script without recording; Record makes each scene its own OBS take,
  // started from the pill.
  const open = (mode: Mode) =>
    run("Opening prompter…", async () => {
      await startSession(mode);
      await openTeleprompter(true);
    });

  return (
    <section className="space-y-4">
      <header>
        <p className="text-xs uppercase tracking-widest text-indigo-400">Phase 0 · Step 3</p>
        <h2 className="text-lg font-semibold">AI script from the running app</h2>
        <p className="mt-1 text-sm text-zinc-400">
          Gemini reads the page's headings and buttons, writes the narration, and pins each click to
          a word. The prompter reads it out and Scenra clicks as you reach each word.
        </p>
      </header>

      {status && (
        <p className={`rounded-lg p-3 text-sm ${status.configured ? "bg-zinc-900 text-zinc-300" : "bg-amber-950 text-amber-200"}`}>
          {status.configured ? (
            <>
              Gemini <code>{status.model}</code> · key from{" "}
              {status.source === "credential-manager" ? "Windows Credential Manager" : "SCENRA_GEMINI_API_KEY"}
            </>
          ) : (
            <>
              No Gemini key. Store it with <code>cmdkey /generic:Scenra/gemini /user:gemini /pass:&lt;key&gt;</code>, then restart Scenra.
            </>
          )}
        </p>
      )}

      <div className="flex gap-2">
        <label className="sr-only" htmlFor="url">App URL</label>
        <input
          id="url"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          className="min-w-0 flex-1 rounded-md bg-zinc-900 px-3 py-1.5 text-sm ring-1 ring-zinc-700 focus:ring-indigo-500 focus:outline-none"
        />
        <Button
          disabled={!!busy}
          onClick={() =>
            run("Launching Chrome…", async () => {
              const profile = await launchChrome(url);
              const cdp = profile.capabilities.cdp;
              setNote(cdp.available ? `Chrome attached · cdp: ${cdp.flavor} · ${cdp.testIds ?? 0} data-testid elements` : `Chrome started, but CDP isn't available: ${profile.warnings.join("; ")}`);
            })
          }
        >
          Launch Chrome
        </Button>
      </div>

      <div className="space-y-2">
        <label htmlFor="brief" className="text-sm text-zinc-400">Brief</label>
        <textarea
          id="brief"
          rows={2}
          value={brief}
          onChange={(e) => setBrief(e.target.value)}
          className="w-full rounded-md bg-zinc-900 px-3 py-2 text-sm ring-1 ring-zinc-700 focus:ring-indigo-500 focus:outline-none"
        />
        <div className="flex flex-wrap gap-2">
          <Button
            disabled={!!busy || !status?.configured}
            onClick={() => run("Drafting with Gemini… (about 10–30 s)", async () => setScript(await draftScript(brief)))}
          >
            Draft script with Gemini
          </Button>
          <Button disabled={!!busy || !script} onClick={() => open("rehearse")}>
            Rehearse
          </Button>
          <Button disabled={!!busy || !script} onClick={() => open("record")}>
            Record scenes
          </Button>
        </div>
      </div>

      {busy && <p className="text-sm text-indigo-300">{busy}</p>}
      {note && <p className="text-sm text-emerald-300">{note}</p>}
      {error && <p className="rounded-lg bg-red-950 p-3 text-sm text-red-200">{error}</p>}

      {script && (
        <div className="space-y-3 rounded-lg bg-zinc-900 p-4 text-sm">
          <p className="font-semibold">{script.title}</p>
          {script.warnings.length > 0 && (
            <ul className="list-disc pl-5 text-amber-300">
              {script.warnings.map((w) => <li key={w}>{w}</li>)}
            </ul>
          )}
          <ol className="space-y-3">
            {script.scenes.map((scene, s) => (
              <li key={s}>
                <p className="text-xs font-bold tracking-wide text-zinc-400 uppercase">
                  {s + 1} · {scene.title}
                </p>
                <ul className="mt-1 space-y-1 font-serif">
                  {scene.lines.map((line, l) => (
                    <li key={l}>
                      {words(line).map((w, i) => {
                        const action = line.actions.find((a) => a.word === i);
                        return action ? (
                          <span key={i} title={action.label} className="border-b-2 border-indigo-400">
                            {w}{" "}
                          </span>
                        ) : (
                          <span key={i}>{w} </span>
                        );
                      })}
                      {line.actions.map((a, i) => (
                        <span key={i} className="ml-1 rounded-full bg-indigo-600 px-2 py-0.5 font-sans text-[11px] font-bold">
                          {a.label}
                        </span>
                      ))}
                    </li>
                  ))}
                </ul>
              </li>
            ))}
          </ol>
        </div>
      )}
    </section>
  );
}
