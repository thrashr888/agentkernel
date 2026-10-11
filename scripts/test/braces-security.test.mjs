import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const braces = require("braces");
const micromatch = require("micromatch");
const fastGlob = require("fast-glob");
const methods = ["parse", "compile", "expand", "stringify", "create"];

function pattern(depth, kind = "brace") {
  const opening = Array.from({ length: depth }, (_, i) =>
    kind === "paren" || (kind === "mixed" && i % 2) ? "(" : "{",
  );
  return opening.join("") + "x" + opening.reverse().map((c) => c === "(" ? ")" : "}").join("");
}

function ast(depth, root = true) {
  const tree = { type: root ? "root" : "paren", nodes: [] };
  let node = tree;
  for (let i = root ? 0 : 1; i < depth; i++) {
    const child = { type: "paren", nodes: [], parent: node };
    node.nodes.push(child);
    node = child;
  }
  node.nodes.push({ type: "text", value: "x" });
  return tree;
}

test("all consumers resolve the source-verified local braces backport", () => {
  const installed = path.dirname(require.resolve("braces/package.json"));
  const vendored = fileURLToPath(new URL("../vendor/braces/", import.meta.url));
  for (const file of ["index.js", "package.json", "LICENSE", "AGENTKERNEL-BACKPORT.md", ...readdirSync(path.join(vendored, "lib")).map((name) => `lib/${name}`)]) {
    assert.equal(readFileSync(path.join(installed, file), "utf8"), readFileSync(path.join(vendored, file), "utf8"), file);
  }
  const daytonaRequire = createRequire(require.resolve("@daytona/sdk"));
  const fastGlobRequire = createRequire(daytonaRequire.resolve("fast-glob"));
  const micromatchRequire = createRequire(fastGlobRequire.resolve("micromatch"));
  assert.equal(micromatchRequire.resolve("braces"), require.resolve("braces"));
  const lock = JSON.parse(readFileSync(new URL("../package-lock.json", import.meta.url)));
  const entries = Object.entries(lock.packages).filter(([name]) => name.endsWith("node_modules/braces"));
  assert.equal(entries.length, 1);
  assert.equal(entries[0][1].resolved, "file:vendor/braces");
  assert.equal(entries[0][1].version, "3.0.3");
});

for (const kind of ["brace", "paren", "mixed"]) {
  test(`${kind} nesting accepts 100 and rejects 101 through every string API`, () => {
    for (const method of methods) {
      assert.doesNotThrow(() => braces[method](pattern(100, kind)), method);
      assert.throws(() => braces[method](pattern(101, kind)), {
        name: "SyntaxError", message: "Input depth (101), exceeds max depth (100)",
      }, method);
    }
    assert.throws(() => braces(pattern(101, kind)), /Input depth/);
    assert.throws(() => braces(pattern(101, kind), { expand: true }), /Input depth/);
  });

  test(`${kind} nesting honors stricter finite limits, including fractions`, () => {
    for (const maxDepth of [0, 0.5, 1, 1.5, 2, 2.5, 99.5]) {
      const allowed = Math.floor(maxDepth);
      for (const method of methods) {
        assert.doesNotThrow(() => braces[method](pattern(allowed, kind), { maxDepth }));
        assert.throws(() => braces[method](pattern(allowed + 1, kind), { maxDepth }), /Input depth/);
      }
    }
    assert.throws(() => braces.parse(pattern(1, kind), { maxDepth: -1 }), /Input depth/);
  });
}

test("non-finite, non-numeric and larger maxDepth values cannot disable the ceiling", () => {
  for (const maxDepth of [NaN, Infinity, -Infinity, undefined, null, "1", false, 101, 10000]) {
    for (const method of methods) {
      assert.doesNotThrow(() => braces[method](pattern(100), { maxDepth }));
      assert.throws(() => braces[method](pattern(101), { maxDepth }), /Input depth/);
    }
  }
});

test("closed siblings release combined depth; unclosed and mixed delimiters remain bounded", () => {
  assert.doesNotThrow(() => braces.parse(`${pattern(100)}${pattern(100, "paren")}`));
  for (const input of ["{".repeat(101), "(".repeat(101), "{(".repeat(51), "{)".repeat(101), "(}".repeat(101)]) {
    assert.throws(() => braces.parse(input), /Input depth/);
  }
});

for (const method of ["compile", "expand", "stringify"]) {
  test(`${method} guards direct root and subtree ASTs`, () => {
    for (const root of [true, false]) {
      assert.doesNotThrow(() => braces[method](ast(100, root)));
      for (const depth of [101, 10000]) {
        assert.throws(() => braces[method](ast(depth, root)), {
          name: "RangeError", message: "AST depth (101), exceeds max depth (100)",
        });
      }
      for (const maxDepth of [0, 0.5, 1, 1.5, 2.5]) {
        if (root || maxDepth >= 1) {
          assert.doesNotThrow(() => braces[method](ast(Math.floor(maxDepth), root), { maxDepth }));
        }
        assert.throws(() => braces[method](ast(Math.floor(maxDepth) + 1, root), { maxDepth }), /AST depth/);
      }
      for (const maxDepth of [NaN, Infinity, -Infinity, 10000]) {
        assert.throws(() => braces[method](ast(101, root), { maxDepth }), /AST depth/);
      }
    }
    const cycle = { type: "root", nodes: [] };
    cycle.nodes.push(cycle);
    assert.throws(() => braces[method](cycle), /AST depth/);
  });
}

test("expand rejects one-node and multi-node parent cycles without hanging", () => {
  execFileSync(process.execPath, ["--input-type=commonjs", "-e", `
    const assert = require('node:assert/strict');
    const braces = require(${JSON.stringify(require.resolve("braces"))});
    for (const length of [1, 2, 100]) {
      const chain = Array.from({length}, () => ({ type: 'paren', nodes: [] }));
      chain.forEach((node, i) => node.parent = chain[(i + 1) % length]);
      const root = { type: 'root', nodes: [chain[0]] };
      assert.throws(() => braces.expand(root), {
        name: 'RangeError', message: 'AST parent chain contains a cycle'
      });
    }
  `], { timeout: 5000, stdio: "pipe" });
});

test("expand's literal-subtree fallback shares the remaining AST depth budget", () => {
  for (const flag of ["invalid", "dollar", "ranges"]) {
    for (const maxDepth of [1.5, 2.5, 100]) {
      const allowed = Math.floor(maxDepth);
      for (const fallbackDepth of [1, allowed]) {
        for (const type of ["paren", "root"]) {
          const tree = (depth) => {
            const root = ast(depth);
            let subtree = root;
            for (let i = 0; i < fallbackDepth; i++) subtree = subtree.nodes[0];
            subtree.type = type;
            subtree[flag] = flag === "ranges" ? 1 : true;
            return root;
          };
          assert.deepEqual(braces.expand(tree(allowed), { maxDepth }), ["x"]);
          assert.throws(() => braces.expand(tree(allowed + 1), { maxDepth }), /AST depth/);
        }
      }
    }
  }
});

test("quoted, escaped and bracketed delimiters are literal rather than AST depth", () => {
  const deep = pattern(101);
  for (const input of [`"${deep}"`, `'${deep}'`, `\`${deep}\``, `[${deep}]`, deep.replace(/[{}]/g, "\\$&")]) {
    for (const method of methods) assert.doesNotThrow(() => braces[method](input));
  }
  assert.equal(braces.compile('foo"{a,b}'), "foo{a,b}");
  assert.deepEqual(braces.expand("'{a,b}"), ["{a,b}"]);
  assert.equal(braces.stringify('"{a,b}"', { keepQuotes: true }), '"{a,b}"');
  assert.equal(braces.stringify("\\{a,b\\}", { keepEscaping: true }), "\\{a,b\\}");
});

test("malformed and escapeInvalid behavior remains compatible with published 3.0.3", () => {
  assert.equal(braces.compile("{a...b}", { escapeInvalid: true }), "\\{a...b\\}");
  // stringify deliberately does not pass parent during recursive traversal.
  assert.equal(braces.stringify("{a...b}", { escapeInvalid: true }), "{a...b}");
  assert.deepEqual(braces.expand("{a...b}", { escapeInvalid: true }), ["{a...b}"]);
  assert.equal(braces.compile("a{b,c", { escapeInvalid: true }), "a\\{b,c");
  assert.deepEqual(braces.expand("(a{b,c}"), ["(ab", "(ac"]);
  assert.equal(braces.compile("${a,b}"), "${a,b}");
  assert.deepEqual(braces.expand("{a..b,c}"), ["a..b", "c"]);
  // Do not adopt PR #72's unrelated comma reset of block.invalid.
  assert.equal(braces.compile("{a..b..c..d,e}"), "{a..b..c..d,e}");
  assert.deepEqual(braces.expand("{a..b..c..d,e}"), ["{a..b..c..d,e}"]);
});

test("micromatch retains ordinary matching and cannot expand excessive nesting", () => {
  assert.deepEqual(micromatch(["src/a.js", "src/b.ts", "src/c.txt"], "src/*.{js,ts}"), ["src/a.js", "src/b.ts"]);
  assert.deepEqual(micromatch.braceExpand("file-{01..03}.{js,ts}"), ["file-01.js", "file-01.ts", "file-02.js", "file-02.ts", "file-03.js", "file-03.ts"]);
  assert.throws(() => micromatch.braceExpand(pattern(101)), /Input depth/);
});

test("fast-glob retains sync and async brace patterns and rejects excessive nesting", async () => {
  const cwd = mkdtempSync(path.join(tmpdir(), "agentkernel-braces-"));
  try {
    mkdirSync(path.join(cwd, "src"));
    for (const name of ["a.js", "b.ts", "c.txt"]) writeFileSync(path.join(cwd, "src", name), "");
    const expected = ["src/a.js", "src/b.ts"];
    assert.deepEqual(fastGlob.sync("src/*.{js,ts}", { cwd }).sort(), expected);
    assert.deepEqual((await fastGlob("src/*.{js,ts}", { cwd })).sort(), expected);
    assert.throws(() => fastGlob.sync(pattern(101), { cwd }), /Input depth/);
    await assert.rejects(async () => fastGlob(pattern(101), { cwd }), /Input depth/);
  } finally {
    rmSync(cwd, { recursive: true, force: true });
  }
});
