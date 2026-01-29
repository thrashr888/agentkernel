import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Health check
console.log("Health:", await client.health());

// Run a command
const result = await client.run(["echo", "Hello from agentkernel!"]);
console.log("Output:", result.output);

// List sandboxes
const sandboxes = await client.listSandboxes();
console.log("Sandboxes:", sandboxes);
