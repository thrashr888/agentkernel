import Foundation

/// Parses Server-Sent Events from a URLSession byte stream.
struct SSEParser {
    /// Parse an async byte stream into StreamEvent values.
    static func parse(
        bytes: URLSession.AsyncBytes
    ) -> AsyncThrowingStream<StreamEvent, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                var eventType = ""
                var dataBuffer = ""

                do {
                    for try await line in bytes.lines {
                        if line.hasPrefix("event:") {
                            eventType = String(line.dropFirst(6)).trimmingCharacters(in: .whitespaces)
                        } else if line.hasPrefix("data:") {
                            let raw = String(line.dropFirst(5)).trimmingCharacters(in: .whitespaces)
                            dataBuffer += raw
                        } else if line.isEmpty {
                            // Empty line = end of event
                            if !eventType.isEmpty || !dataBuffer.isEmpty {
                                let parsed = parseData(dataBuffer)
                                let event = StreamEvent(
                                    eventType: eventType.isEmpty ? "message" : eventType,
                                    data: parsed
                                )
                                continuation.yield(event)
                                eventType = ""
                                dataBuffer = ""
                            }
                        }
                    }
                    // Flush remaining
                    if !eventType.isEmpty || !dataBuffer.isEmpty {
                        let parsed = parseData(dataBuffer)
                        let event = StreamEvent(
                            eventType: eventType.isEmpty ? "message" : eventType,
                            data: parsed
                        )
                        continuation.yield(event)
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: AgentKernelError.stream(error.localizedDescription))
                }
            }

            continuation.onTermination = { _ in
                task.cancel()
            }
        }
    }

    private static func parseData(_ raw: String) -> [String: Any] {
        guard let data = raw.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return ["raw": raw]
        }
        return json
    }
}
