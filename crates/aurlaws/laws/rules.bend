# SPDX-License-Identifier: GPL-2.0-or-later
# The checker: laws over a graph of facts. The parsers and the path and host tables decide what a
# node is; this decides what the shape of the graph means. See archicture/adr/0022-aur-laws.md.
import Base

type Hop is Data:
  Fetches{}
  Writes{}
  Reads{}
  Executes{}
  Decodes{}
  Sends{}
  Verifies{}

type Tag is Data:
  Net{}
  Secret{}
  Outside{}
  Autostart{}
  Foreign{}
  Dynamic{}
  Tracking{}
  OffHome{}
  NoSum{}
  WrongSum{}
  Scriptlet{}
  Upstream{}
  Perms{}
  Caps{}
  Packaged{}
  Copies{}
  NewEdge{}
  Unsaid{}
  Gap{}
  Removes{}

type Node is Data:
  Node{id: U32, tags: +List<Tag>}

type Edge is Data:
  Edge{src: Node, dst: Node, hop: Hop, seq: U32}

# `listed`: the built package's file list is known.
type Graph is Data:
  Graph{nodes: +List<Node>, edges: +List<Edge>, listed: Bool}

type Law is Data:
  FetchExecuteUnverified{}
  DecodeExecute{}
  UnresolvedDynamic{}
  WriteOutsidePackage{}
  ReadCredentials{}
  CredentialToNetwork{}
  ScriptletUndeclaredFile{}
  SourceOffUpstream{}
  UndeclaredPersistence{}
  PostinstallNetwork{}
  BinChecksumUpstream{}
  UnexplainedNewEdge{}
  ObservedMismatch{}
  IncompleteEvidence{}

# Two answers, and neither one is "safe".
type Verdict is Data:
  NeedsReview{broken: +List<Law>}
  NoLawBroken{}

def Tag.code(t: Tag) -> U32:
  match t:
    case Net{}: 0
    case Secret{}: 1
    case Outside{}: 2
    case Autostart{}: 3
    case Foreign{}: 4
    case Dynamic{}: 5
    case Tracking{}: 6
    case OffHome{}: 7
    case NoSum{}: 8
    case WrongSum{}: 9
    case Scriptlet{}: 10
    case Upstream{}: 11
    case Perms{}: 12
    case Caps{}: 13
    case Packaged{}: 14
    case Copies{}: 15
    case NewEdge{}: 16
    case Unsaid{}: 17
    case Gap{}: 18
    case Removes{}: 19

def Tag.in(+t: U32, ts: +List<Tag>) -> Bool:
  match ts:
    case Nil{}:
      False{}
    case Con{h, r}:
      Bool.or(U32.is_eq(t, Tag.code(h)), Tag.in(t, r))

def has(t: Tag, n: Node) -> Bool:
  match n:
    case Node{_, ts}:
      Tag.in(Tag.code(t), ts)

def Hop.flows(h: Hop) -> Bool:
  match h:
    case Fetches{}: True{}
    case Writes{}: True{}
    case Reads{}: True{}
    case Executes{}: True{}
    case Decodes{}: True{}
    case Sends{}: False{}
    case Verifies{}: False{}

def Hop.fetches(h: Hop) -> Bool:
  match h:
    case Fetches{}: True{}
    case _: False{}

def Hop.writes(h: Hop) -> Bool:
  match h:
    case Writes{}: True{}
    case _: False{}

def Hop.reads(h: Hop) -> Bool:
  match h:
    case Reads{}: True{}
    case Decodes{}: True{}
    case _: False{}

def Hop.executes(h: Hop) -> Bool:
  match h:
    case Executes{}: True{}
    case _: False{}

def Hop.decodes(h: Hop) -> Bool:
  match h:
    case Decodes{}: True{}
    case _: False{}

def Hop.sends(h: Hop) -> Bool:
  match h:
    case Sends{}: True{}
    case _: False{}

def Hop.verifies(h: Hop) -> Bool:
  match h:
    case Verifies{}: True{}
    case _: False{}

def Node.id(n: Node) -> U32:
  match n:
    case Node{i, _}:
      i

def Edge.dst_id(e: Edge) -> U32:
  match e:
    case Edge{_, d, _, _}:
      Node.id(d)

def Id.in(+x: U32, xs: +List<U32>) -> Bool:
  match xs:
    case Nil{}:
      False{}
    case Con{h, r}:
      Bool.or(U32.is_eq(x, h), Id.in(x, r))

# A check counts only if it came first.
def checked(es: +List<Edge>, +n: U32, +seq: U32) -> Bool:
  match es:
    case Nil{}:
      False{}
    case Con{Edge{_, d, h, s}, r}:
      Bool.or(Bool.and(Hop.verifies(h), Bool.and(U32.is_eq(n, Node.id(d)), (s < seq : U32))), checked(r, n, seq))

# What a backward walk looks for, and what it may not cross.
type Search is Data:
  Unchecked{seq: U32}
  Decoded{}
  Secrets{}

def stops(m: Search, +all: +List<Edge>, +n: Node) -> Bool:
  match m:
    case Unchecked{s}:
      Bool.or(checked(all, Node.id(n), s), has(Tracking{}, n))
    case _:
      False{}

def goal(m: Search, +src: Node, h: Hop) -> Bool:
  match m:
    case Unchecked{_}:
      Bool.and(has(Net{}, src), Hop.fetches(h))
    case Decoded{}:
      Hop.decodes(h)
    case Secrets{}:
      has(Secret{}, src)

type Pass is Data:
  Pass{found: Bool, seen: +List<U32>, grew: Bool}

def Pass.add(known: Bool, f: Bool, x: U32, seen: +List<U32>, g: Bool) -> Pass:
  match known:
    case True{}:
      Pass{f, seen, g}
    case False{}:
      Pass{f, x <> seen, True{}}

def Pass.step(hit: Bool, +s: Node, h: Hop, +m: Search, f: Bool, +seen: +List<U32>, g: Bool) -> Pass:
  match hit:
    case False{}:
      Pass{f, seen, g}
    case True{}:
      Pass.add(Id.in(Node.id(s), seen), Bool.or(f, goal(m, s, h)), Node.id(s), seen, g)

# Cheap tests first, each decided before the next is worked out.
def Pass.far(known: Bool, +s: Node, h: Hop, +m: Search, +all: +List<Edge>, p: Pass) -> Pass:
  match known:
    case False{}:
      p
    case True{}:
      match p:
        case Pass{f, seen, g}:
          Pass.step(Bool.not(stops(m, all, s)), s, h, m, f, seen, g)

def Pass.near(flow: Bool, +s: Node, +d: Node, h: Hop, +m: Search, +all: +List<Edge>, +p: Pass) -> Pass:
  match flow:
    case False{}:
      p
    case True{}:
      match p:
        case Pass{f, +seen, g}:
          Pass.far(Id.in(Node.id(d), seen), s, h, m, all, Pass{f, seen, g})

def Pass.go(e: Edge, +m: Search, +all: +List<Edge>, p: Pass) -> Pass:
  match e:
    case Edge{+s, +d, +h, _}:
      Pass.near(Hop.flows(h), s, d, h, m, all, p)

def pass(es: +List<Edge>, +m: Search, +all: +List<Edge>, p: Pass) -> Pass:
  match es:
    case Nil{}:
      p
    case Con{e, r}:
      pass(r, m, all, Pass.go(e, m, all, p))

# Walk back until the goal is met or nothing new turns up.
def walk(fuel: Nat, +m: Search, +all: +List<Edge>, r: Pass) -> Bool:
  match fuel:
    case 0n:
      False{}
    case 1n+p:
      match r:
        case Pass{f, +seen, g}:
          Bool.or(f, Bool.and(g, walk(p, m, all, pass(all, m, all, Pass{False{}, seen, False{}}))))

def reaches(+m: Search, +all: +List<Edge>, +from: Node) -> Bool:
  walk(1n+List.length(&2, Edge, all), m, all, pass(all, m, all, Pass{False{}, [Node.id(from)], False{}}))

# Each law, on one edge.
def fetch_run(+all: +List<Edge>, e: Edge) -> Bool:
  match e:
    case Edge{+s, _, Executes{}, +seq}:
      Bool.and(Bool.not(Bool.or(checked(all, Node.id(s), seq), has(Tracking{}, s))), Bool.or(has(Net{}, s), reaches(Unchecked{seq}, all, s)))
    case _:
      False{}

def decode_run(+all: +List<Edge>, e: Edge) -> Bool:
  match e:
    case Edge{+s, _, Executes{}, _}:
      reaches(Decoded{}, all, s)
    case _:
      False{}

def write_outside(e: Edge) -> Bool:
  match e:
    case Edge{+s, d, h, _}:
      Bool.and(Hop.writes(h), Bool.and(has(Outside{}, d), Bool.not(Bool.or(has(Scriptlet{}, s), has(Upstream{}, s)))))

def read_secret(e: Edge) -> Bool:
  match e:
    case Edge{s, _, h, _}:
      Bool.and(Hop.reads(h), has(Secret{}, s))

def secret_out(+all: +List<Edge>, e: Edge) -> Bool:
  match e:
    case Edge{+s, _, Sends{}, _}:
      reaches(Secrets{}, all, s)
    case _:
      False{}

def scriptlet_file(+listed: Bool, e: Edge) -> Bool:
  match e:
    case Edge{+s, d, h, _}:
      Bool.and(Hop.writes(h), Bool.and(has(Foreign{}, d), Bool.and(has(Scriptlet{}, s), Bool.or(listed, Bool.not(Bool.or(has(Perms{}, s), has(Caps{}, s)))))))

def autostart(e: Edge) -> Bool:
  match e:
    case Edge{+s, +d, h, _}:
      Bool.and(Hop.writes(h), Bool.and(has(Autostart{}, d), Bool.and(Bool.not(Bool.or(has(Perms{}, s), has(Removes{}, s))), Bool.not(Bool.and(has(Packaged{}, d), has(Copies{}, s))))))

def online(e: Edge) -> Bool:
  match e:
    case Edge{+s, +d, h, _}:
      match h:
        case Fetches{}:
          has(Scriptlet{}, d)
        case Sends{}:
          has(Scriptlet{}, s)
        case _:
          False{}

def new_edge(e: Edge) -> Bool:
  match e:
    case Edge{+s, +d, _, _}:
      Bool.or(has(NewEdge{}, s), has(NewEdge{}, d))

def said_nothing(e: Edge) -> Bool:
  match e:
    case Edge{+s, +d, h, _}:
      match h:
        case Reads{}:
          Bool.and(has(Secret{}, s), has(Unsaid{}, s))
        case Writes{}:
          Bool.and(has(Unsaid{}, d), Bool.or(has(Outside{}, d), has(Autostart{}, d)))
        case Sends{}:
          has(Unsaid{}, d)
        case _:
          False{}

def on_edge(l: Law, +all: +List<Edge>, +listed: Bool, e: Edge) -> Bool:
  match l:
    case FetchExecuteUnverified{}: fetch_run(all, e)
    case DecodeExecute{}: decode_run(all, e)
    case WriteOutsidePackage{}: write_outside(e)
    case ReadCredentials{}: read_secret(e)
    case CredentialToNetwork{}: secret_out(all, e)
    case ScriptletUndeclaredFile{}: scriptlet_file(listed, e)
    case UndeclaredPersistence{}: autostart(e)
    case PostinstallNetwork{}: online(e)
    case UnexplainedNewEdge{}: new_edge(e)
    case ObservedMismatch{}: said_nothing(e)
    case _: False{}

def any_edge(es: +List<Edge>, +l: Law, +all: +List<Edge>, +listed: Bool) -> Bool:
  match es:
    case Nil{}:
      False{}
    case Con{e, r}:
      Bool.or(on_edge(l, all, listed, e), any_edge(r, l, all, listed))

def any_tag(+t: U32, ns: +List<Node>) -> Bool:
  match ns:
    case Nil{}:
      False{}
    case Con{Node{_, ts}, r}:
      Bool.or(Tag.in(t, ts), any_tag(t, r))

# Dynamic text is a finding by itself, unless a path into it is already the finding.
def runs_into.if(here: Bool, +e: Edge, +all: +List<Edge>) -> Bool:
  match here:
    case False{}:
      False{}
    case True{}:
      Bool.or(fetch_run(all, e), decode_run(all, e))

def runs_into(+n: U32, es: +List<Edge>, +all: +List<Edge>) -> Bool:
  match es:
    case Nil{}:
      False{}
    case Con{+e, r}:
      Bool.or(runs_into.if(U32.is_eq(n, Edge.dst_id(e)), e, all), runs_into(n, r, all))

def unexplained.if(d: Bool, +n: U32, +all: +List<Edge>) -> Bool:
  match d:
    case False{}:
      False{}
    case True{}:
      Bool.not(runs_into(n, all, all))

def unexplained(ns: +List<Node>, +all: +List<Edge>) -> Bool:
  match ns:
    case Nil{}:
      False{}
    case Con{+n, r}:
      Bool.or(unexplained.if(has(Dynamic{}, n), Node.id(n), all), unexplained(r, all))

def edges_break(+l: Law, g: Graph) -> Bool:
  match g:
    case Graph{_, +es, +listed}:
      any_edge(es, l, es, listed)

def nodes_of(g: Graph) -> +List<Node>:
  match g:
    case Graph{ns, _, _}:
      ns

def edges_of(g: Graph) -> +List<Edge>:
  match g:
    case Graph{_, es, _}:
      es

def broken(l: Law, +g: Graph) -> Bool:
  match l:
    case UnresolvedDynamic{}: unexplained(nodes_of(g), edges_of(g))
    case SourceOffUpstream{}: any_tag(7, nodes_of(g))
    case BinChecksumUpstream{}: Bool.or(any_tag(8, nodes_of(g)), any_tag(9, nodes_of(g)))
    case IncompleteEvidence{}: any_tag(18, nodes_of(g))
    case _: edges_break(l, g)

def keep.if(b: Bool, l: Law, rest: +List<Law>) -> +List<Law>:
  match b:
    case True{}:
      l <> rest
    case False{}:
      rest

def Law.all() -> +List<Law>:
  [FetchExecuteUnverified{}, DecodeExecute{}, UnresolvedDynamic{}, WriteOutsidePackage{}, ReadCredentials{}, CredentialToNetwork{}, ScriptletUndeclaredFile{}, SourceOffUpstream{}, UndeclaredPersistence{}, PostinstallNetwork{}, BinChecksumUpstream{}, UnexplainedNewEdge{}, ObservedMismatch{}, IncompleteEvidence{}]

def keep(+l: Law, +g: Graph, rest: +List<Law>) -> +List<Law>:
  keep.if(broken(l, g), l, rest)

def check.go(ls: +List<Law>, +g: Graph) -> +List<Law>:
  match ls:
    case Nil{}:
      []
    case Con{l, r}:
      keep(l, g, check.go(r, g))

# Every broken law, in the order above.
def check(+g: Graph) -> +List<Law>:
  check.go(Law.all(), g)

def verdict.of(ls: +List<Law>) -> Verdict:
  match ls:
    case Nil{}:
      NoLawBroken{}
    case Con{+h, +t}:
      NeedsReview{h <> t}

def verdict(+g: Graph) -> Verdict:
  verdict.of(check(g))
