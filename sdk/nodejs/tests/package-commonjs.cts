import agentkernel = require("agentkernel");

const options: agentkernel.AgentKernelOptions = {
  baseUrl: "http://127.0.0.1:18999",
};
const client = new agentkernel.AgentKernel(options);

void client;
void agentkernel.BrowserSession;
void agentkernel.SandboxSession;
