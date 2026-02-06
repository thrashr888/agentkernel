# Railway deployment Dockerfile for agentkernel
FROM ghcr.io/thrashr888/agentkernel:latest

# Railway provides PORT env var
ENV PORT=18888

EXPOSE $PORT

CMD ["agentkernel", "serve", "--host", "0.0.0.0"]
