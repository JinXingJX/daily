let sdk;
export default {
  activate(host) { sdk = host; },
  views: {
    "writer-lab.canvas": { mount(root) {
  let draft = "";
  let busy = false;
  let output = "Open this canvas from the Daily sidebar.";

  root.innerHTML = `
    <main class="shell">
      <aside class="rail">
        <div class="kicker">Writer Lab</div>
        <h1>Branch Canvas</h1>
        <button id="save">Save Draft</button>
        <button id="model" class="primary">Ask Model</button>
        <button id="agent">Run Agent</button>
      </aside>
      <section class="canvas">
        <div class="branch">
          <span>Worldview</span>
          <strong>Generate setting, rules, factions, and central conflict.</strong>
        </div>
        <div class="branch alt">
          <span>Storyline</span>
          <strong>Expand branches, compare choices, then turn one branch into chapters.</strong>
        </div>
        <div class="branch">
          <span>Sample</span>
          <strong>Draft the first scene and revise it through the host bridge.</strong>
        </div>
      </section>
      <section class="editor">
        <textarea id="draft" aria-label="Story idea" placeholder="Describe the story you want to build..."></textarea>
        <pre id="output" role="status" aria-live="polite"></pre>
      </section>
    </main>
    <style>
      * { box-sizing: border-box; }
      :host { color: var(--text); background: var(--app-main-bg); font: 14px/20px var(--font-sans); }
      .shell { display: grid; grid-template-columns: 220px minmax(0,1fr) 340px; height: 100cqh; overflow: hidden; }
      .rail { border-right: 1px solid var(--line); padding: 22px 16px; background: var(--app-sidebar-bg); }
      .kicker { color: var(--text-muted); font-size: 12px; }
      h1 { margin: 8px 0 28px; font: 400 30px/38px var(--font-serif); color: var(--text-strong); }
      button { display: flex; align-items: center; justify-content: center; width: 100%; min-height: 36px; margin: 8px 0; border: 1px solid var(--line-strong); border-radius: var(--radius-control); background: var(--surface-strong); color: var(--text-strong); padding: 6px 12px; cursor: pointer; font: 500 13px/20px var(--font-sans); }
      button:hover { background: var(--fill-hover); }button:disabled{opacity:.5;cursor:not-allowed}
      button.primary{background:var(--primary);border-color:var(--primary);color:var(--primary-contrast)}
      button.primary:hover{background:var(--primary-hover)}
      :focus-visible{outline:0;box-shadow:var(--focus-ring)}
      .canvas { padding: 32px; overflow: auto; }
      .branch { margin: 0 0 28px; border-bottom: 1px solid var(--line); padding: 0 0 22px; }
      .branch span { display: block; color: var(--text-muted); font-size: 12px; }
      .branch strong { display: block; margin-top: 8px; font-weight: 500; line-height: 24px; }
      .editor { display: grid; min-height:0; grid-template-rows: minmax(160px,1fr) minmax(160px,1fr); border-left: 1px solid var(--line); }
      textarea { width: 100%; min-width:0; height:100%; resize:none; border:0; border-bottom:1px solid var(--line); background:var(--surface-strong); color:var(--text-strong); padding:20px; font:400 15px/24px var(--font-sans); }
      pre { margin:0; overflow:auto; white-space:pre-wrap; overflow-wrap:anywhere; padding:20px; color:var(--text); font:400 16px/27px var(--font-reading); }
      @container daily-view (max-width:900px) { .shell { grid-template-columns:200px minmax(0,1fr); }.canvas{display:none} }
      @container daily-view (max-width:530px) { .shell { grid-template-columns:1fr;grid-template-rows:auto minmax(0,1fr); }.rail{padding:16px;border:0;border-bottom:1px solid var(--line)}h1{font-size:24px;margin:4px 0 12px}.rail button{display:inline-flex;width:auto;margin:0 4px 0 0}.editor{border:0} }
    </style>
  `;

  const draftEl = root.querySelector("#draft");
  const outputEl = root.querySelector("#output");

  function render() {
    draftEl.value = draft;
    outputEl.textContent = output;
    root.querySelectorAll("button").forEach(button => { button.disabled = busy || (button.id !== "save" && !draft.trim()); });
  }

  async function boot() {
    try {
      draft = (await sdk.storage.get("draft")) || "";
      const context = await sdk.host.getContext();
      output = `Connected to ${context.plugin.name}. Available agents: ${(await sdk.agent.list()).length}`;
    } catch (error) {
      output = error.message;
    }
    render();
  }

  draftEl.addEventListener("input", function () {
    draft = draftEl.value;
    void sdk.host.setDirty(true);
    render();
  });

  root.querySelector("#save").addEventListener("click", async function () {
    try {
      await sdk.storage.set("draft", draft);
      await sdk.host.setDirty(false);
      output = "Draft saved to plugin-private project storage.";
    } catch (error) {
      output = error.message;
    }
    render();
  });

  root.querySelector("#model").addEventListener("click", async function () {
    if (busy || !draft.trim()) return;
    busy = true;
    output = "Thinking...";
    render();
    try {
      const result = await sdk.model.invoke({
        system: "You help professional fiction writers create structured long-form stories.",
        prompt: `Create a world bible, story outline, and first chapter sample from this idea:\n\n${draft}`,
        maxOutputTokens: 1200
      });
      output = result.text;
    } catch (error) {
      output = error.message;
    }
    busy = false;
    render();
  });

  root.querySelector("#agent").addEventListener("click", async function () {
    if (busy || !draft.trim()) return;
    busy = true;
    output = "Running agent...";
    render();
    try {
      const result = await sdk.agent.run({
        prompt: "Turn the current writing idea into three branching plot directions.",
        context: draft
      });
      output = `${result.agentName}\n\n${result.text}`;
    } catch (error) {
      output = error.message;
    }
    busy = false;
    render();
  });

  boot();
    return () => root.replaceChildren();
    } },
  },
};
