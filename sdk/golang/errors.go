package agentkernel

import "fmt"

// Error represents an agentkernel API error.
type Error struct {
	StatusCode int
	Message    string
}

func (e *Error) Error() string {
	return fmt.Sprintf("agentkernel: %s (status %d)", e.Message, e.StatusCode)
}

// IsAuthError returns true if the error is an authentication error.
func IsAuthError(err error) bool {
	if e, ok := err.(*Error); ok {
		return e.StatusCode == 401
	}
	return false
}

// IsValidationError returns true if the error is a validation error.
func IsValidationError(err error) bool {
	if e, ok := err.(*Error); ok {
		return e.StatusCode == 400
	}
	return false
}

// IsNotFoundError returns true if the error is a not-found error.
func IsNotFoundError(err error) bool {
	if e, ok := err.(*Error); ok {
		return e.StatusCode == 404
	}
	return false
}

// IsServerError returns true if the error is a server error.
func IsServerError(err error) bool {
	if e, ok := err.(*Error); ok {
		return e.StatusCode >= 500
	}
	return false
}

func errorFromStatus(status int, message string) *Error {
	if message == "" {
		switch {
		case status == 400:
			message = "bad request"
		case status == 401:
			message = "unauthorized"
		case status == 404:
			message = "not found"
		case status >= 500:
			message = "internal server error"
		default:
			message = "unknown error"
		}
	}
	return &Error{StatusCode: status, Message: message}
}
