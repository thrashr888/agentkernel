/**
 * Simple HTTP server in TypeScript for demonstrating agentkernel.
 */

import { createServer, IncomingMessage, ServerResponse } from 'http';

const PORT = process.env.PORT || 3000;

interface HealthResponse {
  status: string;
  timestamp: string;
}

function handleRequest(req: IncomingMessage, res: ServerResponse): void {
  if (req.url === '/health') {
    const health: HealthResponse = {
      status: 'ok',
      timestamp: new Date().toISOString(),
    };
    res.writeHead(200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(health));
  } else {
    res.writeHead(200, { 'Content-Type': 'text/plain' });
    res.end('Hello from agentkernel sandbox!');
  }
}

const server = createServer(handleRequest);

server.listen(PORT, () => {
  console.log(`TypeScript server listening on port ${PORT}`);
});
