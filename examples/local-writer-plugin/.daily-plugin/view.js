let sdk;
export default {
  activate(host) { sdk = host; },
  views: {
    "writer-lab.canvas": { mount(root) {
  let draft = "";
  let output = "Open this canvas from the Daily sidebar.";

  root.innerHTML = `
    <main class="shell">
      <aside class="rail">
        <div class="kicker">Writer Lab</div>
        <h1>Branch Canvas</h1>
        <button id="save">Save Draft</button>
        <button id="model">Ask Model</button>
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
        <textarea id="draft" placeholder="Describe the story you want to build..."></textarea>
        <pre id="output"></pre>
      </section>
    </main>
    <style>
      * { box-sizing: border-box; }
      :host { margin: 0; background: #111; color: #f4f4f5; }
      .shell { display: grid; grid-template-columns: 18rem minmax(18rem, 1fr) minmax(22rem, 34rem); height: 100cqh; }
      .rail { border-right: 1px solid #2a2a2f; padding: 24px; background: #18181b; }
      .kicker { color: #a1a1aa; font-size: 11px; font-weight: 700; letter-spacing: 0; text-transform: uppercase; }
      h1 { margin: 8px 0 24px; font-size: 28px; line-height: 1.1; }
      button { display: flex; width: 100%; margin: 10px 0; border: 1px solid #3f3f46; border-radius: 8px; background: #27272a; color: #fafafa; padding: 10px 12px; cursor: pointer; }
      button:hover { background: #34343a; }
      .canvas { position: relative; padding: 32px; background: #101010; overflow: auto; }
      .canvas:before { content: ""; position: absolute; inset: 0; background-image: linear-gradient(#27272a 1px, transparent 1px), linear-gradient(90deg, #27272a 1px, transparent 1px); background-size: 44px 44px; opacity: .24; }
      .branch { position: relative; max-width: 28rem; margin: 0 0 28px; border: 1px solid #3f3f46; border-radius: 8px; background: #18181bcc; padding: 18px; }
      .branch.alt { margin-left: 64px; }
      .branch span { display: block; color: #a1a1aa; font-size: 12px; font-weight: 700; text-transform: uppercase; }
      .branch strong { display: block; margin-top: 8px; line-height: 1.5; }
      .editor { display: grid; grid-template-rows: minmax(12rem, 1fr) minmax(12rem, 1fr); border-left: 1px solid #2a2a2f; background: #18181b; }
      textarea { width: 100%; height: 100%; resize: none; border: 0; border-bottom: 1px solid #2a2a2f; outline: none; background: #18181b; color: #fafafa; padding: 20px; line-height: 1.6; }
      pre { margin: 0; overflow: auto; white-space: pre-wrap; padding: 20px; color: #d4d4d8; line-height: 1.6; }
      @container daily-view (max-width: 900px) { .shell { grid-template-columns: 1fr; grid-template-rows: auto 22rem minmax(24rem, 1fr); } .rail, .editor { border: 0; } }
    </style>
  `;

  const draftEl = root.querySelector("#draft");
  const outputEl = root.querySelector("#output");

  function render() {
    draftEl.value = draft;
    outputEl.textContent = output;
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
  });

  root.querySelector("#save").addEventListener("click", async function () {
    try {
      await sdk.storage.set("draft", draft);
      output = "Draft saved to plugin-private project storage.";
    } catch (error) {
      output = error.message;
    }
    render();
  });

  root.querySelector("#model").addEventListener("click", async function () {
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
    render();
  });

  root.querySelector("#agent").addEventListener("click", async function () {
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
    render();
  });

  boot();
    return () => root.replaceChildren();
    } },
  },
};
