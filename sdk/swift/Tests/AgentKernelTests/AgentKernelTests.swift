import Foundation
import XCTest

@testable import AgentKernel

// MARK: - Mock URLProtocol

final class MockURLProtocol: URLProtocol {
    static var requestHandler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let handler = MockURLProtocol.requestHandler else {
            XCTFail("No request handler set")
            return
        }
        do {
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

// MARK: - Helpers

/// Read request body from either httpBody or httpBodyStream.
/// URLSession converts httpBody to httpBodyStream before passing to URLProtocol.
func readBody(_ request: URLRequest) -> Data? {
    if let body = request.httpBody {
        return body
    }
    guard let stream = request.httpBodyStream else { return nil }
    stream.open()
    defer { stream.close() }
    var data = Data()
    let bufferSize = 4096
    let buffer = UnsafeMutablePointer<UInt8>.allocate(capacity: bufferSize)
    defer { buffer.deallocate() }
    while stream.hasBytesAvailable {
        let read = stream.read(buffer, maxLength: bufferSize)
        if read <= 0 { break }
        data.append(buffer, count: read)
    }
    return data
}

func bodyJSON(_ request: URLRequest) -> [String: Any]? {
    guard let data = readBody(request) else { return nil }
    return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
}

func makeClient(baseURL: String = "http://localhost:9999", apiKey: String? = nil) -> AgentKernel {
    let config = URLSessionConfiguration.ephemeral
    config.protocolClasses = [MockURLProtocol.self]
    return AgentKernel(
        options: AgentKernelOptions(baseURL: baseURL, apiKey: apiKey),
        sessionConfiguration: config
    )
}

func jsonResponse(_ json: String, status: Int = 200) -> (HTTPURLResponse, Data) {
    let response = HTTPURLResponse(
        url: URL(string: "http://localhost:9999")!,
        statusCode: status,
        httpVersion: "HTTP/1.1",
        headerFields: ["Content-Type": "application/json"]
    )!
    return (response, json.data(using: .utf8)!)
}

// MARK: - Tests

final class AgentKernelTests: XCTestCase {

    override func tearDown() {
        MockURLProtocol.requestHandler = nil
        super.tearDown()
    }

    // MARK: Health

    func testHealth() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { _ in
            jsonResponse(#"{"success":true,"data":"ok"}"#)
        }
        let result = try await client.health()
        XCTAssertEqual(result, "ok")
    }

    // MARK: Run

    func testRun() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertTrue(request.url!.path.hasSuffix("/run"))

            let body = bodyJSON(request)!
            XCTAssertEqual(body["command"] as! [String], ["echo", "hello"])
            XCTAssertEqual(body["fast"] as! Bool, true)

            return jsonResponse(#"{"success":true,"data":{"output":"hello\n"}}"#)
        }
        let output = try await client.run(["echo", "hello"])
        XCTAssertEqual(output.output, "hello\n")
    }

    func testRunWithOptions() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            let body = bodyJSON(request)!
            XCTAssertEqual(body["image"] as? String, "python:3.12")
            XCTAssertEqual(body["profile"] as? String, "restrictive")
            XCTAssertEqual(body["fast"] as? Bool, false)

            return jsonResponse(#"{"success":true,"data":{"output":"done"}}"#)
        }
        let opts = RunOptions(image: "python:3.12", profile: .restrictive, fast: false)
        let output = try await client.run(["python", "-c", "print('hi')"], options: opts)
        XCTAssertEqual(output.output, "done")
    }

    // MARK: Sandboxes

    func testListSandboxes() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { _ in
            jsonResponse(#"{"success":true,"data":[{"name":"sb1","status":"running","backend":"docker"}]}"#)
        }
        let sandboxes = try await client.listSandboxes()
        XCTAssertEqual(sandboxes.count, 1)
        XCTAssertEqual(sandboxes[0].name, "sb1")
        XCTAssertEqual(sandboxes[0].status, "running")
    }

    func testCreateSandbox() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            let body = bodyJSON(request)!
            XCTAssertEqual(body["name"] as? String, "test-sb")

            return jsonResponse(#"{"success":true,"data":{"name":"test-sb","status":"running","backend":"docker"}}"#)
        }
        let sb = try await client.createSandbox("test-sb")
        XCTAssertEqual(sb.name, "test-sb")
        XCTAssertEqual(sb.backend, "docker")
    }

    func testGetSandbox() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            XCTAssertTrue(request.url!.path.hasSuffix("/sandboxes/my-sb"))
            return jsonResponse(#"{"success":true,"data":{"name":"my-sb","status":"running","backend":"docker"}}"#)
        }
        let sb = try await client.getSandbox("my-sb")
        XCTAssertEqual(sb.name, "my-sb")
    }

    func testRemoveSandbox() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "DELETE")
            XCTAssertTrue(request.url!.path.hasSuffix("/sandboxes/my-sb"))
            return jsonResponse(#"{"success":true,"data":"removed"}"#)
        }
        try await client.removeSandbox("my-sb")
    }

    func testExecInSandbox() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            XCTAssertEqual(request.httpMethod, "POST")
            XCTAssertTrue(request.url!.path.hasSuffix("/sandboxes/test-sb/exec"))
            let body = bodyJSON(request)!
            XCTAssertEqual(body["command"] as! [String], ["ls", "-la"])

            return jsonResponse(#"{"success":true,"data":{"output":"total 0\n"}}"#)
        }
        let output = try await client.execInSandbox("test-sb", command: ["ls", "-la"])
        XCTAssertEqual(output.output, "total 0\n")
    }

    // MARK: withSandbox

    func testWithSandboxCleansUp() async throws {
        let client = makeClient()
        var requestPaths: [String] = []

        MockURLProtocol.requestHandler = { request in
            requestPaths.append("\(request.httpMethod!) \(request.url!.path)")

            if request.httpMethod == "DELETE" {
                return jsonResponse(#"{"success":true,"data":"removed"}"#)
            }
            if request.url!.path.hasSuffix("/exec") {
                return jsonResponse(#"{"success":true,"data":{"output":"result"}}"#)
            }
            // create
            return jsonResponse(#"{"success":true,"data":{"name":"tmp","status":"running","backend":"docker"}}"#)
        }

        let result: String = try await client.withSandbox("tmp") { session in
            let output = try await session.run(["echo", "hi"])
            return output.output
        }

        XCTAssertEqual(result, "result")
        XCTAssertTrue(requestPaths.contains("POST /sandboxes"))
        XCTAssertTrue(requestPaths.contains("DELETE /sandboxes/tmp"))
    }

    func testWithSandboxCleansUpOnError() async throws {
        let client = makeClient()
        var deleteCalled = false

        MockURLProtocol.requestHandler = { request in
            if request.httpMethod == "DELETE" {
                deleteCalled = true
                return jsonResponse(#"{"success":true,"data":"removed"}"#)
            }
            if request.url!.path.hasSuffix("/exec") {
                return jsonResponse(#"{"error":"exec failed"}"#, status: 500)
            }
            return jsonResponse(#"{"success":true,"data":{"name":"tmp","status":"running","backend":"docker"}}"#)
        }

        do {
            let _: String = try await client.withSandbox("tmp") { session in
                let output = try await session.run(["bad"])
                return output.output
            }
            XCTFail("Should have thrown")
        } catch {
            // Expected
        }
        XCTAssertTrue(deleteCalled, "Sandbox should be removed even on error")
    }

    // MARK: Error Mapping

    func testAuthError() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { _ in
            jsonResponse(#"{"error":"invalid key"}"#, status: 401)
        }
        do {
            _ = try await client.health()
            XCTFail("Should have thrown")
        } catch let error as AgentKernelError {
            if case .auth(let msg) = error {
                XCTAssertEqual(msg, "invalid key")
            } else {
                XCTFail("Wrong error type: \(error)")
            }
        }
    }

    func testValidationError() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { _ in
            jsonResponse(#"{"error":"bad request"}"#, status: 400)
        }
        do {
            _ = try await client.run([""])
            XCTFail("Should have thrown")
        } catch let error as AgentKernelError {
            if case .validation(let msg) = error {
                XCTAssertEqual(msg, "bad request")
            } else {
                XCTFail("Wrong error type: \(error)")
            }
        }
    }

    func testNotFoundError() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { _ in
            jsonResponse(#"{"error":"sandbox not found"}"#, status: 404)
        }
        do {
            _ = try await client.getSandbox("nonexistent")
            XCTFail("Should have thrown")
        } catch let error as AgentKernelError {
            if case .notFound(let msg) = error {
                XCTAssertEqual(msg, "sandbox not found")
            } else {
                XCTFail("Wrong error type: \(error)")
            }
        }
    }

    func testServerError() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { _ in
            jsonResponse(#"{"error":"internal failure"}"#, status: 500)
        }
        do {
            _ = try await client.health()
            XCTFail("Should have thrown")
        } catch let error as AgentKernelError {
            if case .server(let msg) = error {
                XCTAssertEqual(msg, "internal failure")
            } else {
                XCTFail("Wrong error type: \(error)")
            }
        }
    }

    // MARK: Headers

    func testUserAgentHeader() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            let ua = request.value(forHTTPHeaderField: "User-Agent")
            XCTAssertTrue(ua?.hasPrefix("agentkernel-swift-sdk/") == true)
            return jsonResponse(#"{"success":true,"data":"ok"}"#)
        }
        _ = try await client.health()
    }

    func testApiKeyHeader() async throws {
        let client = makeClient(apiKey: "sk-test-123")
        MockURLProtocol.requestHandler = { request in
            let auth = request.value(forHTTPHeaderField: "Authorization")
            XCTAssertEqual(auth, "Bearer sk-test-123")
            return jsonResponse(#"{"success":true,"data":"ok"}"#)
        }
        _ = try await client.health()
    }

    func testNoAuthHeaderWithoutApiKey() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { request in
            XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
            return jsonResponse(#"{"success":true,"data":"ok"}"#)
        }
        _ = try await client.health()
    }

    // MARK: API Failure Response

    func testApiFailureResponse() async throws {
        let client = makeClient()
        MockURLProtocol.requestHandler = { _ in
            jsonResponse(#"{"success":false,"error":"something went wrong"}"#)
        }
        do {
            _ = try await client.health()
            XCTFail("Should have thrown")
        } catch let error as AgentKernelError {
            if case .server(let msg) = error {
                XCTAssertEqual(msg, "something went wrong")
            } else {
                XCTFail("Wrong error type: \(error)")
            }
        }
    }
}
