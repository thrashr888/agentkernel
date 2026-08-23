import {
  AgentKernel,
  BrowserSession,
  SandboxSession,
  type AgentKernelOptions,
} from "agentkernel";

const options: AgentKernelOptions = { baseUrl: "http://127.0.0.1:18999" };
const client = new AgentKernel(options);

void client;
void BrowserSession;
void SandboxSession;
