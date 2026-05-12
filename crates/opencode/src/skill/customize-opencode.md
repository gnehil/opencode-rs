---
name: customize-opencode
description: Learn how to customize opencode configuration for your project
---

# Customizing OpenCode

OpenCode can be customized for your project using `opencode.json` configuration file.

## Configuration Schema

### Basic Structure

```json
{
  "agents": {
    "build": {
      "model": "anthropic/claude-3-5-sonnet",
      "system": "Custom system prompt..."
    }
  },
  "tools": {
    "bash": {
      "timeout": 60000
    }
  },
  "mcp": {
    "servers": {
      "my-server": {
        "url": "https://api.example.com/mcp"
      }
    }
  },
  "permissions": {
    "allow": ["read:*", "bash:ls"],
    "deny": ["bash:rm:*"]
  }
}
```

## Agents Configuration

Define custom agent modes with specific behaviors:

```json
{
  "agents": {
    "review": {
      "model": "anthropic/claude-3-5-sonnet",
      "system": "You are a code reviewer...",
      "tools": ["read", "grep"]
    }
  }
}
```

## MCP Servers

Connect external tools via MCP protocol:

```json
{
  "mcp": {
    "servers": {
      "filesystem": {
        "type": "local",
        "command": "mcp-server-filesystem",
        "args": ["/path/to/dir"]
      }
    }
  }
}
```

## Permissions

Control which tools require approval:

```json
{
  "permissions": {
    "rules": [
      { "tool": "bash", "rule": "allow", "pattern": "git *" },
      { "tool": "write", "rule": "ask", "pattern": "*.rs" }
    ]
  }
}
```

## Skills

Add custom skills to extend capabilities:

```json
{
  "skills": {
    "paths": ["./skills"],
    "urls": ["https://skills.example.com/index.json"]
  }
}
```

Create skill files as `SKILL.md` in skill directories:

```markdown
---
name: my-skill
description: Description of what this skill does
---

Skill content goes here...
```