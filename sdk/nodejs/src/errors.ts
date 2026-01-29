/** Base error for all agentkernel SDK errors. */
export class AgentKernelError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AgentKernelError";
  }
}

/** 401 Unauthorized. */
export class AuthError extends AgentKernelError {
  readonly status = 401;
  constructor(message = "Unauthorized") {
    super(message);
    this.name = "AuthError";
  }
}

/** 404 Not Found. */
export class NotFoundError extends AgentKernelError {
  readonly status = 404;
  constructor(message = "Not found") {
    super(message);
    this.name = "NotFoundError";
  }
}

/** 400 Bad Request. */
export class ValidationError extends AgentKernelError {
  readonly status = 400;
  constructor(message = "Bad request") {
    super(message);
    this.name = "ValidationError";
  }
}

/** 500 Internal Server Error. */
export class ServerError extends AgentKernelError {
  readonly status = 500;
  constructor(message = "Internal server error") {
    super(message);
    this.name = "ServerError";
  }
}

/** Network / connection error. */
export class NetworkError extends AgentKernelError {
  constructor(message = "Network error") {
    super(message);
    this.name = "NetworkError";
  }
}

/** SSE streaming error. */
export class StreamError extends AgentKernelError {
  constructor(message = "Stream error") {
    super(message);
    this.name = "StreamError";
  }
}

/** Map an HTTP status code + body to the appropriate error. */
export function errorFromStatus(status: number, body: string): AgentKernelError {
  let message: string;
  try {
    const parsed = JSON.parse(body);
    message = parsed.error ?? body;
  } catch {
    message = body;
  }

  switch (status) {
    case 400:
      return new ValidationError(message);
    case 401:
      return new AuthError(message);
    case 404:
      return new NotFoundError(message);
    default:
      return new ServerError(message);
  }
}
