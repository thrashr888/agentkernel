import java.io.*;
import java.net.*;
import java.time.Instant;

/**
 * Simple HTTP server in Java for demonstrating agentkernel.
 */
public class Server {
    private static final int PORT = Integer.parseInt(
        System.getenv().getOrDefault("PORT", "8080")
    );

    public static void main(String[] args) throws IOException {
        ServerSocket serverSocket = new ServerSocket(PORT);
        System.out.println("Java server listening on port " + PORT);

        while (true) {
            try (Socket clientSocket = serverSocket.accept();
                 BufferedReader in = new BufferedReader(
                     new InputStreamReader(clientSocket.getInputStream())
                 );
                 PrintWriter out = new PrintWriter(
                     clientSocket.getOutputStream(), true
                 )) {

                String requestLine = in.readLine();
                if (requestLine == null) continue;

                String response;
                String contentType;

                if (requestLine.contains("/health")) {
                    contentType = "application/json";
                    response = String.format(
                        "{\"status\": \"ok\", \"timestamp\": \"%s\"}",
                        Instant.now().toString()
                    );
                } else {
                    contentType = "text/plain";
                    response = "Hello from agentkernel sandbox!";
                }

                out.println("HTTP/1.1 200 OK");
                out.println("Content-Type: " + contentType);
                out.println("Content-Length: " + response.length());
                out.println();
                out.println(response);
            } catch (IOException e) {
                System.err.println("Connection error: " + e.getMessage());
            }
        }
    }
}
