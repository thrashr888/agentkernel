import assert from "node:assert/strict";
import test from "node:test";

import {
  Daytona,
  FileSystem,
  Process,
  PtyHandle,
  Sandbox,
} from "@daytona/sdk";

const bridgeApi = [
  [Daytona, ["create", "get", Symbol.asyncDispose]],
  [Sandbox, ["start", "stop", "delete", "getPreviewLink"]],
  [
    FileSystem,
    [
      "createFolder",
      "listFiles",
      "deleteFile",
      "downloadFile",
      "downloadFiles",
      "uploadFile",
      "uploadFiles",
    ],
  ],
  [Process, ["executeCommand", "createPty"]],
  [
    PtyHandle,
    [
      "waitForConnection",
      "sendInput",
      "resize",
      "wait",
      "disconnect",
    ],
  ],
];

test("maintained Daytona SDK exports the bridge API", () => {
  for (const [type, methods] of bridgeApi) {
    assert.equal(typeof type, "function");
    for (const method of methods) {
      assert.equal(
        typeof type.prototype[method],
        "function",
        `${type.name}.${String(method)} must remain callable`,
      );
    }
  }
});

