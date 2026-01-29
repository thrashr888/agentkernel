package agentkernel

import (
	"bufio"
	"encoding/json"
	"io"
	"strings"
)

// ParseSSE reads SSE events from r and sends them on the returned channel.
// The channel is closed when the stream ends or an error occurs.
// Errors are returned as StreamEvent with Type "error".
func ParseSSE(r io.Reader) <-chan StreamEvent {
	ch := make(chan StreamEvent)
	go func() {
		defer close(ch)
		scanner := bufio.NewScanner(r)
		var eventType string
		for scanner.Scan() {
			line := scanner.Text()
			switch {
			case strings.HasPrefix(line, "event: "):
				eventType = strings.TrimPrefix(line, "event: ")
			case strings.HasPrefix(line, "data: "):
				dataStr := strings.TrimPrefix(line, "data: ")
				var data map[string]interface{}
				if err := json.Unmarshal([]byte(dataStr), &data); err != nil {
					data = map[string]interface{}{"raw": dataStr}
				}
				typ := eventType
				if typ == "" {
					typ = "message"
				}
				ch <- StreamEvent{Type: typ, Data: data}
				eventType = ""
			case line == "":
				// Empty line separates events, reset
				eventType = ""
			}
		}
	}()
	return ch
}
