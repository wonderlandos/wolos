---
title: "Let a script or an AI helper use Atlas"
section: "Developers"
order: 3.2
summary: "Atlas Drive lets a script, a test or an AI helper open pages, click, type and read results in Atlas, in small answers a model can afford, and it asks you once per helper, never per click."
---
## What it is

**Atlas Drive** (`atlas-drive`, part of the Developer {{tip:module}}) lets a program use Atlas the way you do: open a page, click a button, fill a form, take a screenshot, read what changed. It is for tests (the WolOS hub and your own web apps), for scripts, and for AI helpers that need a browser. It has no network port: it talks to Atlas through a pipe it opens itself, and only programs of your own account can reach it.

## Why WolOS does it this way

An AI helper pays for every word it reads. A page dumped as HTML is tens of thousands of words; Atlas Drive gives the helper a few hundred lines: one line for each button, field, link and heading, each with a short name (`e12`) it can act on. After a click it answers with what changed (the address, the title, a dialog, errors), and it can send the change of the page in the same answer. Text on a page that a person cannot see, or that reads like an instruction to the helper, is held back and reported on its own, so a page cannot quietly talk to your helper.

## How

### Look at a page and act on it

```
atlas-drive goto https://example.org
atlas-drive snapshot
atlas-drive click e3 --then diff
```

The first command starts Atlas (without a window) if it is not running. `snapshot` prints the page; `click e3` presses the element named `e3` there and, with `--then diff`, prints only what changed. You can also name an element by what it says: `atlas-drive click "Save"` or `atlas-drive click "role=button name~=Save"`.

### Give a helper the short manual

`atlas-drive --help-agent` prints one page meant to be pasted into an assistant's instructions. `atlas-drive mcp` starts a server that assistants speaking MCP can use directly (the same way `wol mcp` works).

### Decide what a helper may do

The first time a helper connects, you allow it once, for what it needs: **read** pages, **act** (click, type, go to addresses), use the **network** tools, use **storage** (cookies), or use **files** (downloads, uploads, PDFs). You are not asked again per click.

```
atlas-drive permissions list
atlas-drive permissions allow /usr/bin/my-agent --scopes read,act
atlas-drive permissions revoke /usr/bin/my-agent
atlas-drive permissions auto on --for 2h --site example.org
atlas-drive permissions auto off
```

**Auto** mode stops the questions for a time, or for a site, and you can turn it off at any moment. While a helper is connected, a note in the session says so.

### If something goes wrong

Every message is one line with the cause and what to do. "unknown ref e9" means the page changed since the last `snapshot`: run it again. "is not allowed to use 'act'" means you have not allowed that helper yet: run the `permissions allow` line it prints.
