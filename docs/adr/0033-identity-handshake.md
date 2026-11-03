# 0033 - wold greets first: identity handshake on the control socket
Status: accepted. Builds on 0020 (identity from the kernel). Closes audit-security A2 (identity after parse).

## Context
wold decides grants on who the kernel says is on the other end of the control socket
(`/var/run/wold.sock`): SO_PEERCRED's uid and pid, and the executable of that pid. Since v0.12 the
identity is captured at accept and pinned with a `/proc/<pid>` descriptor (`crates/api/src/peer.rs`),
and the executable is read again before any grant, so a peer that exec()s *after* accept is refused.
One gap was left: a socket accepts data before the server calls accept(). A process could connect,
write a request, then exec() a trusted binary (setuid or not) before wold accepted. wold would then
read a request the old image wrote, attributed to the new image's executable. A process's pid and
start time survive exec, so neither a pidfd nor `/proc/<pid>/stat` can tell the two apart.

## Decision
- wold writes `hello <nonce>` (128 random bits, hex; `shared::jobs::send_hello`) on every accepted
  control connection before it reads anything. The client's first line must echo that line exactly
  (`shared::jobs::answer_hello`); otherwise the connection is dropped without an answer. Only after
  the echo does wold read the request (`crates/api/src/server.rs`, `serve`).
- The hello is mandatory, deny by default. There is no fallback for clients that do not speak it.
- In-tree clients connect through `shared::jobs::connect` (connect + handshake). Test fakes of wold
  call `send_hello` and skip the echo line.
- Either side waits at most `shared::jobs::HELLO_TIMEOUT` (5 s) for the other's hello.

## Consequences
- A request written before accept can never be the first line after the hello, because the nonce did
  not exist yet. A process that exec()ed before accept cannot answer: the new image does not know
  about the pending connection (Rust and libwol sockets are close-on-exec).
- The control socket costs one extra round trip per connection (one line each way).
- Fork gap closed in userspace (audit S5): a process that forks, keeps the socket in the child, and
  lets the parent (the pid the kernel recorded at connect) exec a trusted binary before accept
  cannot answer the hello from the child. `shared::jobs::greet` turns SO_PASSCRED on before it
  sends the hello, so the kernel stamps every byte the client writes with the writing process
  (SCM_CREDENTIALS; an unprivileged sender can claim only its own pid), and drops the connection
  unless every byte of the echo carries the SO_PEERCRED pid. Clients change nothing. Tests:
  `shared::jobs` `greet_refuses_an_echo_from_another_process`.
- Residual: a client that legitimately forks between connect and its hello (none in tree) is
  refused; it must connect after the fork. A trusted binary whose parent is hostile is ADR 0020's
  concern (audit S6), not the handshake's.
- The memsaver and qos sockets greet the same way (`crates/api/src/bin/wold.rs` calls `greet`), and
  the realtime agent answers through `shared::jobs::answer_hello`.
- `session.approve`: a helper's admin tool (`wol mcp`) no longer dials the shell; it asks wold,
  which names the caller by the executable the kernel reports for it and forwards the sheet, and
  the shell answers an `approve` line only from wold (`crates/controller/src/approve.rs`,
  `crates/shell/src/ui/ipc.rs`).
