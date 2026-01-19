/**
 * Simple HTTP server in C# for demonstrating agentkernel.
 *
 * Run: dotnet run
 */

using System.Net;
using System.Text;
using System.Text.Json;

var port = int.Parse(Environment.GetEnvironmentVariable("PORT") ?? "5000");
var listener = new HttpListener();
listener.Prefixes.Add($"http://+:{port}/");
listener.Start();

Console.WriteLine($"C# server listening on port {port}");

while (true)
{
    var context = await listener.GetContextAsync();
    var request = context.Request;
    var response = context.Response;

    string responseString;
    string contentType;

    if (request.Url?.AbsolutePath == "/health")
    {
        contentType = "application/json";
        responseString = JsonSerializer.Serialize(new
        {
            status = "ok",
            timestamp = DateTime.UtcNow.ToString("o")
        });
    }
    else
    {
        contentType = "text/plain";
        responseString = "Hello from agentkernel sandbox!";
    }

    response.ContentType = contentType;
    var buffer = Encoding.UTF8.GetBytes(responseString);
    response.ContentLength64 = buffer.Length;
    await response.OutputStream.WriteAsync(buffer);
    response.Close();
}
