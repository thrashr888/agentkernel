#!/bin/sh
# Simple HTTP server in pure shell for demonstrating agentkernel.
#
# Uses netcat (nc) which is available on most systems.
# Run: ./server.sh

PORT=${PORT:-8080}

echo "Shell server listening on port $PORT"

while true; do
    # Read request
    REQUEST=$(echo -e "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nHello from agentkernel sandbox!" | nc -l -p $PORT -q 1 2>/dev/null || \
              echo -e "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nHello from agentkernel sandbox!" | nc -l $PORT 2>/dev/null)
done
