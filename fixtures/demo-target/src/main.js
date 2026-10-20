// Plain DOM, no framework: every interactive element has a data-testid for Scenra's CDP locator.
const $ = (id) => document.querySelector(`[data-testid="${id}"]`);

const status = (text) => {
  $("status").textContent = text;
};

window.addEventListener("DOMContentLoaded", () => {
  const panel = $("recording-panel");
  const dialog = $("settings-dialog");
  const title = $("title-input");

  $("create-recording").addEventListener("click", () => {
    panel.hidden = false;
    title.focus();
    status("New recording: enter a title.");
  });

  $("start-recording").addEventListener("click", () => {
    const item = document.createElement("li");
    item.textContent = title.value.trim() || "Untitled recording";
    $("recordings").append(item);
    title.value = "";
    panel.hidden = true;
    status(`Recording "${item.textContent}" created.`);
  });

  $("open-settings").addEventListener("click", () => dialog.showModal());
  $("settings-cancel").addEventListener("click", () => {
    dialog.close();
    status("Settings unchanged.");
  });
  $("settings-confirm").addEventListener("click", () => {
    dialog.close();
    status("Settings saved.");
  });
});
