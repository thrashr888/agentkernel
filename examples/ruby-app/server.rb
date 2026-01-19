# Simple HTTP server in Ruby for demonstrating agentkernel.

require 'socket'
require 'json'

PORT = ENV.fetch('PORT', 4567).to_i

server = TCPServer.new('0.0.0.0', PORT)
puts "Ruby server listening on port #{PORT}"

loop do
  client = server.accept
  request = client.gets

  if request&.include?('/health')
    response = { status: 'ok', timestamp: Time.now.iso8601 }.to_json
    client.print "HTTP/1.1 200 OK\r\n"
    client.print "Content-Type: application/json\r\n"
    client.print "\r\n"
    client.print response
  else
    client.print "HTTP/1.1 200 OK\r\n"
    client.print "Content-Type: text/plain\r\n"
    client.print "\r\n"
    client.print "Hello from agentkernel sandbox!"
  end

  client.close
end
