import { describe, it, expect, beforeAll, afterAll, afterEach } from "vitest";
import { http, HttpResponse } from "msw";
import { setupServer } from "msw/node";
import { AgentKernel } from "../src/client.js";
import {
  AuthError,
  NotFoundError,
  ValidationError,
  ServerError,
} from "../src/errors.js";

const BASE_URL = "http://localhost:9999";

const handlers = [
  http.get(`${BASE_URL}/health`, () =>
    HttpResponse.json({ success: true, data: "ok" }),
  ),

  http.post(`${BASE_URL}/run`, async ({ request }) => {
    const body = (await request.json()) as { command: string[] };
    return HttpResponse.json({
      success: true,
      data: { output: body.command.join(" ") + "\n" },
    });
  }),

  http.get(`${BASE_URL}/sandboxes`, () =>
    HttpResponse.json({
      success: true,
      data: [
        { name: "test-1", status: "running", backend: "docker" },
        { name: "test-2", status: "stopped", backend: "docker" },
      ],
    }),
  ),

  http.post(`${BASE_URL}/sandboxes`, async ({ request }) => {
    const body = (await request.json()) as { name: string; image?: string };
    return HttpResponse.json(
      {
        success: true,
        data: { name: body.name, status: "running", backend: "docker" },
      },
      { status: 201 },
    );
  }),

  http.get(`${BASE_URL}/sandboxes/my-sandbox`, () =>
    HttpResponse.json({
      success: true,
      data: { name: "my-sandbox", status: "running", backend: "docker" },
    }),
  ),

  http.get(`${BASE_URL}/sandboxes/not-found`, () =>
    HttpResponse.json(
      { success: false, error: "Sandbox not found" },
      { status: 404 },
    ),
  ),

  http.delete(`${BASE_URL}/sandboxes/my-sandbox`, () =>
    HttpResponse.json({ success: true, data: "Sandbox removed" }),
  ),

  http.post(`${BASE_URL}/sandboxes/my-sandbox/exec`, async ({ request }) => {
    const body = (await request.json()) as { command: string[] };
    return HttpResponse.json({
      success: true,
      data: { output: body.command.join(" ") + "\n" },
    });
  }),
];

const server = setupServer(...handlers);

beforeAll(() => server.listen({ onUnhandledRequest: "error" }));
afterEach(() => server.resetHandlers());
afterAll(() => server.close());

function client(opts?: { apiKey?: string }) {
  return new AgentKernel({ baseUrl: BASE_URL, ...opts });
}

describe("AgentKernel", () => {
  describe("health", () => {
    it("returns ok", async () => {
      expect(await client().health()).toBe("ok");
    });
  });

  describe("run", () => {
    it("runs a command and returns output", async () => {
      const result = await client().run(["echo", "hello"]);
      expect(result.output).toBe("echo hello\n");
    });

    it("passes options through", async () => {
      server.use(
        http.post(`${BASE_URL}/run`, async ({ request }) => {
          const body = (await request.json()) as Record<string, unknown>;
          expect(body.image).toBe("python:3.12-alpine");
          expect(body.profile).toBe("restrictive");
          expect(body.fast).toBe(false);
          return HttpResponse.json({
            success: true,
            data: { output: "ok\n" },
          });
        }),
      );
      await client().run(["python3", "-c", "print('ok')"], {
        image: "python:3.12-alpine",
        profile: "restrictive",
        fast: false,
      });
    });
  });

  describe("listSandboxes", () => {
    it("returns sandbox list", async () => {
      const list = await client().listSandboxes();
      expect(list).toHaveLength(2);
      expect(list[0].name).toBe("test-1");
      expect(list[0].status).toBe("running");
    });
  });

  describe("createSandbox", () => {
    it("creates a sandbox", async () => {
      const sb = await client().createSandbox("new-sb");
      expect(sb.name).toBe("new-sb");
      expect(sb.status).toBe("running");
    });
  });

  describe("getSandbox", () => {
    it("returns sandbox info", async () => {
      const sb = await client().getSandbox("my-sandbox");
      expect(sb.name).toBe("my-sandbox");
    });

    it("throws NotFoundError for missing sandbox", async () => {
      await expect(client().getSandbox("not-found")).rejects.toThrow(
        NotFoundError,
      );
    });
  });

  describe("removeSandbox", () => {
    it("removes a sandbox", async () => {
      await expect(client().removeSandbox("my-sandbox")).resolves.toBeUndefined();
    });
  });

  describe("execInSandbox", () => {
    it("executes in a sandbox", async () => {
      const result = await client().execInSandbox("my-sandbox", [
        "echo",
        "test",
      ]);
      expect(result.output).toBe("echo test\n");
    });
  });

  describe("sandbox session", () => {
    it("creates and auto-removes", async () => {
      let removed = false;
      server.use(
        http.delete(`${BASE_URL}/sandboxes/session-test`, () => {
          removed = true;
          return HttpResponse.json({ success: true, data: "Sandbox removed" });
        }),
      );

      const c = client();
      const sb = await c.sandbox("session-test");
      expect(sb.name).toBe("session-test");
      await sb.remove();
      expect(removed).toBe(true);
    });

    it("remove is idempotent", async () => {
      let removeCount = 0;
      server.use(
        http.delete(`${BASE_URL}/sandboxes/idem-test`, () => {
          removeCount++;
          return HttpResponse.json({ success: true, data: "Sandbox removed" });
        }),
      );

      const sb = await client().sandbox("idem-test");
      await sb.remove();
      await sb.remove();
      expect(removeCount).toBe(1);
    });
  });

  describe("authentication", () => {
    it("sends Bearer token when apiKey is set", async () => {
      server.use(
        http.get(`${BASE_URL}/health`, ({ request }) => {
          expect(request.headers.get("authorization")).toBe("Bearer sk-test");
          return HttpResponse.json({ success: true, data: "ok" });
        }),
      );
      await client({ apiKey: "sk-test" }).health();
    });
  });

  describe("error handling", () => {
    it("throws AuthError on 401", async () => {
      server.use(
        http.get(`${BASE_URL}/health`, () =>
          HttpResponse.json(
            { success: false, error: "Unauthorized" },
            { status: 401 },
          ),
        ),
      );
      await expect(client().health()).rejects.toThrow(AuthError);
    });

    it("throws ValidationError on 400", async () => {
      server.use(
        http.post(`${BASE_URL}/run`, () =>
          HttpResponse.json(
            { success: false, error: "Invalid command" },
            { status: 400 },
          ),
        ),
      );
      await expect(client().run([])).rejects.toThrow(ValidationError);
    });

    it("throws ServerError on 500", async () => {
      server.use(
        http.get(`${BASE_URL}/health`, () =>
          HttpResponse.json(
            { success: false, error: "Internal error" },
            { status: 500 },
          ),
        ),
      );
      await expect(client().health()).rejects.toThrow(ServerError);
    });
  });

  describe("user-agent", () => {
    it("sends user-agent header", async () => {
      server.use(
        http.get(`${BASE_URL}/health`, ({ request }) => {
          expect(request.headers.get("user-agent")).toMatch(
            /^agentkernel-nodejs-sdk\//,
          );
          return HttpResponse.json({ success: true, data: "ok" });
        }),
      );
      await client().health();
    });
  });
});
