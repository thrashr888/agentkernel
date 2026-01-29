import { AgentKernel } from "agentkernel";

const client = new AgentKernel();

// Create a sandbox session — auto-removed when scope exits
await using sb = await client.sandbox("demo", { image: "python:3.12-alpine" });

// Install a package
await sb.run(["pip", "install", "numpy"]);

// Run code
const result = await sb.run([
  "python3",
  "-c",
  "import numpy; print(f'numpy {numpy.__version__}')",
]);
console.log(result.output);

// Sandbox is automatically removed here
