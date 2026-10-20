// The magenta band is the marker `probe_capture` looks for. Keep it #FF00FF (see capture.rs).
export default function Teleprompter() {
  return (
    <div className="flex h-full flex-col">
      <div className="bg-[#ff00ff] px-5 py-3 text-sm font-semibold tracking-wide text-black">
        SCENRA TELEPROMPTER · must not appear in the recording
      </div>
      <p className="flex-1 overflow-auto px-6 py-5 font-serif text-2xl leading-relaxed">
        Now let's create a new recording. Lumina lets you record meetings and automatically
        generate structured notes. <span className="rounded bg-indigo-500/30 px-1">Click New
        Recording</span>, give it a title, and we're ready to go.
      </p>
    </div>
  );
}
