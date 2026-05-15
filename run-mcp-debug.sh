#!/bin/bash
# Wrapper for MCP debug — redirects stderr to /tmp/researcher-mcp-debug.log
exec "$(dirname "$0")/target/release/researcher-mcp" "$@" 2>>/tmp/researcher-mcp-debug.log
