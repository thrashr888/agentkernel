import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Stream output from a command
for await (const event of client.runStream(["python3", "-c", "print('Hello from streaming!')"])) {
  switch (event.type) {
    case "started":
      console.log("[started]", event.data);
      break;
    case "output":
      process.stdout.write(String(event.data.data));
      break;
    case "done":
      console.log("\n[done] exit_code:", event.data.exit_code);
      break;
    case "error":
      console.error("[error]", event.data.message);
      break;
  }
}
