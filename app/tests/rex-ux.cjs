(async () => {
  const assert = require("node:assert/strict");
  const fs = require("node:fs");
  const vm = require("node:vm");
  const { transformWithOxc } = await import("vite");
  const { test } = require("node:test");
  const full = fs.readFileSync("src/components/sandbox/create-rex-sandbox.tsx", "utf8");
  const compiled = (await transformWithOxc(
    full.slice(full.indexOf("function validManualRexUrl"), full.indexOf("function RexCreateDialog"))
      .replace("export function", "function"),
    "rex-ux-fixture.tsx", { jsx: { runtime: "classic" } },
  )).code;
  const rexAddress = "unix:///private/tmp/ak-r_12/exact/rex.sock";

  function fixture({ availability = {}, saving = false, clipboard = true, openScope = null } = {}) {
    const calls = [];
    const context = vm.createContext({
      React: { createElement: (type, props) => ({ type, props }) },
      RexCreateDialog: "RexCreateDialog",
      api: {},
      navigator: { clipboard: clipboard ? { writeText: async (text) => calls.push(text) } : null },
      useSettings: () => ({ settings: { active_server: "local", servers: [{ name: "local", url: "http://localhost" }] } }),
      useIsMutating: () => Number(saving),
      useState: () => [openScope, () => {}],
      useQuery: () => availability,
    });
    vm.runInContext(compiled, context);
    return { context, calls };
  }
  test("Rex Create is only shown for an affirmative owner-local response", () => {
    for (const input of [
      { availability: { data: false } }, { availability: {} },
      { availability: { data: true, isError: true } }, { availability: { data: true }, saving: true },
    ]) assert.equal(fixture(input).context.CreateRexSandbox(), null);
    assert.equal(fixture({ availability: { data: true } }).context.CreateRexSandbox().type, "RexCreateDialog");
  });
  test("a failed probe cannot erase an open create result for the same server", () => {
    const scope = JSON.stringify(["local", "http://localhost", undefined]);
    const availability = { data: true, isError: true };
    const retained = fixture({ availability, openScope: scope }).context.CreateRexSandbox();
    assert.equal(retained.type, "RexCreateDialog");
    assert.equal(retained.props.available, false);
    assert.equal(fixture({ availability, openScope: "other" }).context.CreateRexSandbox(), null);
  });
  test("the dialog dispatches only a valid name through its enabled submit path", async () => {
    const rendered = (await transformWithOxc(
      full.slice(full.indexOf("function RexCreateDialog"), full.indexOf("export function PreparedRexSandboxes")),
      "rex-submit-fixture.tsx", { jsx: { runtime: "classic" } },
    )).code;
    const search = (node, tag) => {
      if (!node || typeof node !== "object") return null;
      if (node.type === tag) return node;
      return node.children?.map((child) => search(child, tag)).find(Boolean);
    };
    for (const [name, available, pending, expected] of [
      ["  rex-demo-1  ", true, false, ["rex-demo-1"]],
      ["invalid--name", true, false, []],
      ["rex-demo-1", false, false, []],
      ["rex-demo-1", true, true, []],
    ]) {
      let state = 0;
      const calls = [];
      const context = vm.createContext({
        React: { createElement: (type, props, ...children) => ({ type, props, children }) },
        Dialog: "Dialog", DialogTrigger: "DialogTrigger", DialogContent: "DialogContent",
        DialogHeader: "DialogHeader", DialogTitle: "DialogTitle", DialogDescription: "DialogDescription",
        Label: "Label", Input: "Input", Button: "Button",
        useQueryClient: () => ({}),
        useState: () => [[true, name, ""][state++], () => {}],
        useMutation: () => ({ isPending: pending, mutate: (payload) => calls.push(payload) }),
      });
      vm.runInContext(rendered, context);
      const form = search(context.RexCreateDialog({ available }), "form");
      form.props.onSubmit({ preventDefault: () => {} });
      assert.deepEqual(calls, expected);
    }
  });
  test("a prepared row verifies before displaying an address and requires removal confirmation", async () => {
    const rendered = (await transformWithOxc(full.slice(full.indexOf("function PreparedRexRow")),
      "rex-row-fixture.tsx", { jsx: { runtime: "classic" } })).code;
    const treeText = (node) => !node ? "" : typeof node === "string" ? node :
      Array.isArray(node) ? node.map(treeText).join("") : treeText(node.children);
    const walk = (node, predicate) => !node || typeof node !== "object" ? undefined :
      predicate(node) ? node : (node.children ?? []).map((child) => walk(child, predicate)).find(Boolean);
    function row(status, confirmation, verified, available = true) {
      let state = 0;
      let mutation = 0;
      const calls = [];
      const ctx = vm.createContext({
        React: { createElement: (type, props, ...children) => ({ type, props, children }), Fragment: "Fragment" },
        Button: "Button",
        useQueryClient: () => ({ invalidateQueries: () => {} }),
        useState: () => [[confirmation, ""][state++], (value) => calls.push(value)],
        useMutation: () => ++mutation === 1 ?
          { data: verified ? { rex_url: rexAddress } : undefined, mutate: () => calls.push("reopen") } :
          { mutate: () => calls.push("remove") },
      });
      vm.runInContext(rendered, ctx);
      return { tree: ctx.PreparedRexRow({ box: { name: "test", status }, available }), calls };
    }
    const initial = row("reopenable", false, false);
    assert.doesNotMatch(treeText(initial.tree), /unix:\/\//);
    const findButton = (view, text) => walk(view.tree, (node) => node.type === "Button" && treeText(node) === text);
    findButton(initial, "Get Rex Address").props.onClick();
    assert.deepEqual(initial.calls, ["", "reopen"]);
    findButton(initial, "Remove").props.onClick();
    assert.deepEqual(initial.calls, ["", "reopen", true]);
    assert.match(treeText(row("open", false, true).tree), /unix:\/\//);
    const offline = row("unavailable", false, true);
    assert.equal(findButton(offline, "Get Rex Address").props.disabled, true);
    assert.doesNotMatch(treeText(offline.tree), /unix:\/\//);
    const probeFailed = row("open", true, true, false);
    assert.equal(findButton(probeFailed, "Confirm removal").props.disabled, true);
    assert.doesNotMatch(treeText(probeFailed.tree), /unix:\/\//);
    const confirmation = row("open", true, false);
    findButton(confirmation, "Confirm removal").props.onClick();
    assert.deepEqual(confirmation.calls, ["remove"]);
  });
  test("rejects wrong scheme, authority, encoded or malformed path before copying", async () => {
    const { context, calls } = fixture();
    for (const url of [
      "ssh://sandbox@localhost:3022", "unix://remote/a.sock", "unix:////tmp/a.sock",
      "unix:///tmp/../a.sock", "unix:///tmp/a%2esock", "unix:///tmp/a.sock#f",
      "unix:///tmp/a.sock?x", "unix:///tmp/a.sock\n", "unix:///tmp/a.sock ",
    ]) {
      assert.equal(context.validManualRexUrl(url), false, url);
      await assert.rejects(context.copyManualRexUrl(url));
    }
    assert.deepEqual(calls, []);
  });
  test("copies the exact URL, with no open or enrollment", async () => {
    const { context, calls } = fixture();
    await context.copyManualRexUrl(rexAddress);
    assert.deepEqual(calls, [rexAddress]);
    await assert.rejects(fixture({ clipboard: false }).context.copyManualRexUrl(rexAddress));
  });
})().catch((error) => { console.error(error); process.exitCode = 1; });
