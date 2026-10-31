# SPDX-License-Identifier: GPL-2.0-or-later
# AUR laws: contracts over the graph of a package. Each law says which shape is a finding.
# These establish how the checker behaves on these graphs and, where a law says `for`, on every
# graph. They do not establish that a graph captured what a program does: a broken law means
# "needs review", a clean graph means "no law broken", and neither means anything about safety.
import Base
import ./rules.bend as R

def a() -> R.Node:
  R.Node{1, []}

def b() -> R.Node:
  R.Node{2, []}

def c() -> R.Node:
  R.Node{3, []}

def url() -> R.Node:
  R.Node{9, [R.Net{}]}

def secret() -> R.Node:
  R.Node{8, [R.Secret{}]}

def on(n: R.Node, h: R.Hop, m: R.Node, s: U32) -> R.Edge:
  R.Edge{n, m, h, s}

def graph(es: +List<R.Edge>) -> R.Graph:
  R.Graph{[], es, True{}}

# A download that runs.
def fetched_run() -> R.Graph:
  graph([on(url(), R.Fetches{}, a(), 1), on(a(), R.Writes{}, b(), 2), on(b(), R.Executes{}, c(), 3)])

def fetched_checked_run() -> R.Graph:
  graph([on(url(), R.Fetches{}, a(), 1), on(a(), R.Writes{}, b(), 2), on(a(), R.Verifies{}, b(), 3), on(b(), R.Executes{}, c(), 4)])

def fetched_run_checked_late() -> R.Graph:
  graph([on(url(), R.Fetches{}, a(), 1), on(a(), R.Writes{}, b(), 2), on(b(), R.Executes{}, c(), 3), on(a(), R.Verifies{}, b(), 4)])

def tracked_run() -> R.Graph:
  graph([on(url(), R.Fetches{}, R.Node{4, [R.Tracking{}]}, 1), on(R.Node{4, [R.Tracking{}]}, R.Writes{}, b(), 2), on(b(), R.Executes{}, c(), 3)])

law fetch_execute_unverified_found:
  {R.check(fetched_run()) == [R.FetchExecuteUnverified{}] : List<&2, R.Law>}

law fetch_execute_unverified_clean:
  {R.check(fetched_checked_run()) == [] : List<&2, R.Law>}

law a_check_after_the_run_is_no_check:
  {R.check(fetched_run_checked_late()) == [R.FetchExecuteUnverified{}] : List<&2, R.Law>}

law declared_vcs_tracking_is_not_a_stray_download:
  {R.check(tracked_run()) == [] : List<&2, R.Law>}

# Decoded data never runs.
def decoded_run() -> R.Graph:
  graph([on(a(), R.Decodes{}, b(), 1), on(b(), R.Writes{}, c(), 2), on(c(), R.Executes{}, R.Node{4, []}, 3)])

def decoded_kept() -> R.Graph:
  graph([on(a(), R.Decodes{}, b(), 1), on(b(), R.Writes{}, c(), 2)])

law decode_execute_found:
  {R.check(decoded_run()) == [R.DecodeExecute{}] : List<&2, R.Law>}

law decode_execute_clean:
  {R.check(decoded_kept()) == [] : List<&2, R.Law>}

# Text that cannot be resolved is a finding, not a pass.
def dynamic() -> R.Node:
  R.Node{5, [R.Dynamic{}]}

def unresolved() -> R.Graph:
  R.Graph{[dynamic()], [], True{}}

def unresolved_but_explained() -> R.Graph:
  R.Graph{[dynamic()], [on(url(), R.Executes{}, dynamic(), 1)], True{}}

law unresolved_dynamic_found:
  {R.check(unresolved()) == [R.UnresolvedDynamic{}] : List<&2, R.Law>}

law unresolved_dynamic_clean:
  {R.check(R.Graph{[], [], True{}}) == [] : List<&2, R.Law>}

law a_known_path_into_dynamic_text_is_the_finding:
  {R.check(unresolved_but_explained()) == [R.FetchExecuteUnverified{}] : List<&2, R.Law>}

# Nothing outside $pkgdir and $srcdir is written.
def outside() -> R.Node:
  R.Node{6, [R.Outside{}]}

law write_outside_package_found:
  {R.check(graph([on(a(), R.Writes{}, outside(), 1)])) == [R.WriteOutsidePackage{}] : List<&2, R.Law>}

law write_outside_package_clean:
  {R.check(graph([on(a(), R.Writes{}, b(), 1)])) == [] : List<&2, R.Law>}

law a_scriptlet_is_judged_by_its_own_law:
  {R.check(graph([on(scriptlet(), R.Writes{}, outside(), 1)])) == [] : List<&2, R.Law>}

law upstream_build_files_stay_in_the_tree_they_describe:
  {R.check(graph([on(R.Node{1, [R.Upstream{}]}, R.Writes{}, outside(), 1)])) == [] : List<&2, R.Law>}

# Credentials, browser profiles and SSH keys are not read.
law read_credentials_found:
  {R.check(graph([on(secret(), R.Reads{}, a(), 1)])) == [R.ReadCredentials{}] : List<&2, R.Law>}

law read_credentials_clean:
  {R.check(graph([on(b(), R.Reads{}, a(), 1)])) == [] : List<&2, R.Law>}

# What was read from a credential does not reach the network.
def exfiltrated() -> R.Graph:
  graph([on(secret(), R.Reads{}, a(), 1), on(a(), R.Writes{}, b(), 2), on(b(), R.Reads{}, c(), 3), on(c(), R.Sends{}, url(), 4)])

law credential_to_network_found:
  {R.check(exfiltrated()) == [R.ReadCredentials{}, R.CredentialToNetwork{}] : List<&2, R.Law>}

law credential_to_network_clean:
  {R.check(graph([on(b(), R.Reads{}, c(), 1), on(c(), R.Sends{}, url(), 2)])) == [] : List<&2, R.Law>}

# A .install scriptlet touches only what its package declares.
def scriptlet() -> R.Node:
  R.Node{7, [R.Scriptlet{}]}

def chmod_scriptlet() -> R.Node:
  R.Node{7, [R.Scriptlet{}, R.Perms{}]}

def foreign() -> R.Node:
  R.Node{6, [R.Foreign{}]}

law scriptlet_undeclared_file_found:
  {R.check(graph([on(scriptlet(), R.Writes{}, foreign(), 1)])) == [R.ScriptletUndeclaredFile{}] : List<&2, R.Law>}

law scriptlet_undeclared_file_clean:
  {R.check(graph([on(scriptlet(), R.Writes{}, b(), 1)])) == [] : List<&2, R.Law>}

law a_mode_change_waits_for_the_real_file_list:
  {R.check(R.Graph{[], [on(chmod_scriptlet(), R.Writes{}, foreign(), 1)], False{}}) == [] : List<&2, R.Law>}

# Sources stay on the declared upstream.
law source_off_upstream_found:
  {R.check(R.Graph{[R.Node{1, [R.OffHome{}]}], [], True{}}) == [R.SourceOffUpstream{}] : List<&2, R.Law>}

law source_off_upstream_clean:
  {R.check(R.Graph{[R.Node{1, []}], [], True{}}) == [] : List<&2, R.Law>}

# Nothing starts itself again unless the package ships it.
def autostart() -> R.Node:
  R.Node{6, [R.Autostart{}]}

def shipped_unit() -> R.Node:
  R.Node{6, [R.Autostart{}, R.Packaged{}]}

law undeclared_persistence_found:
  {R.check(graph([on(a(), R.Writes{}, autostart(), 1)])) == [R.UndeclaredPersistence{}] : List<&2, R.Law>}

law undeclared_persistence_clean:
  {R.check(graph([on(R.Node{1, [R.Copies{}]}, R.Writes{}, shipped_unit(), 1)])) == [] : List<&2, R.Law>}

law a_mode_change_starts_nothing:
  {R.check(graph([on(R.Node{1, [R.Perms{}]}, R.Writes{}, autostart(), 1)])) == [] : List<&2, R.Law>}

# No network after the package is installed.
law postinstall_network_found:
  {R.check(graph([on(url(), R.Fetches{}, scriptlet(), 1)])) == [R.PostinstallNetwork{}] : List<&2, R.Law>}

law postinstall_network_clean:
  {R.check(graph([on(url(), R.Fetches{}, a(), 1)])) == [] : List<&2, R.Law>}

# A -bin source is tied to what upstream published.
law bin_checksum_upstream_found:
  {R.check(R.Graph{[R.Node{1, [R.NoSum{}]}], [], True{}}) == [R.BinChecksumUpstream{}] : List<&2, R.Law>}

law bin_checksum_differs_found:
  {R.check(R.Graph{[R.Node{1, [R.WrongSum{}]}], [], True{}}) == [R.BinChecksumUpstream{}] : List<&2, R.Law>}

law bin_checksum_upstream_clean:
  {R.check(R.Graph{[R.Node{1, []}], [], True{}}) == [] : List<&2, R.Law>}

# Between versions, a new edge needs a reason.
law unexplained_new_edge_found:
  {R.check(graph([on(R.Node{1, [R.NewEdge{}]}, R.Reads{}, a(), 1)])) == [R.UnexplainedNewEdge{}] : List<&2, R.Law>}

law unexplained_new_edge_clean:
  {R.check(graph([on(b(), R.Reads{}, a(), 1)])) == [] : List<&2, R.Law>}

# What the run did must be what the package said.
def unsaid_secret() -> R.Node:
  R.Node{8, [R.Secret{}, R.Unsaid{}]}

law observed_mismatch_found:
  {R.check(graph([on(unsaid_secret(), R.Reads{}, a(), 1)])) == [R.ReadCredentials{}, R.ObservedMismatch{}] : List<&2, R.Law>}

law observed_mismatch_clean:
  {R.check(graph([on(secret(), R.Reads{}, a(), 1)])) == [R.ReadCredentials{}] : List<&2, R.Law>}

# What was not read must not read as clean.
law incomplete_evidence_found:
  {R.check(R.Graph{[R.Node{1, [R.Gap{}]}], [], True{}}) == [R.IncompleteEvidence{}] : List<&2, R.Law>}

law incomplete_evidence_clean:
  {R.check(R.Graph{[], [], True{}}) == [] : List<&2, R.Law>}

# The words of the verdict.
law nothing_broken_says_exactly_that:
  {R.verdict(R.Graph{[], [], True{}}) == R.NoLawBroken{} : R.Verdict}

law any_broken_law_asks_for_review:
  for +l: R.Law
  for +ls: +List<R.Law>
  {R.verdict.of(l <> ls) == R.NeedsReview{l <> ls} : R.Verdict}

# These hold for every input, not only for the cases above.
law a_gap_is_never_lost:
  for +ts: +List<R.Tag>
  for +ns: +List<R.Node>
  {R.any_tag(18, R.Node{1, R.Gap{} <> ts} <> ns) == True{} : Bool}

law reading_a_secret_is_always_found:
  for +n: U32
  for +ts: +List<R.Tag>
  for +d: R.Node
  for +s: U32
  {R.read_secret(R.Edge{R.Node{n, R.Secret{} <> ts}, d, R.Reads{}, s}) == True{} : Bool}

law an_unchecked_network_run_is_always_found:
  for +all: +List<R.Edge>
  for +n: U32
  for +d: R.Node
  for +seq: U32
  for h: {R.checked(all, n, seq) == False{} : Bool}
  {R.fetch_run(all, R.Edge{R.Node{n, [R.Net{}]}, d, R.Executes{}, seq}) == True{} : Bool}

law true_absorbs_or:
  for b: Bool
  {Bool.or(b, True{}) == True{} : Bool}

law a_broken_edge_is_found_anywhere_in_the_list:
  for xs: +List<R.Edge>
  for +e: R.Edge
  for +ys: +List<R.Edge>
  for +l: R.Law
  for +all: +List<R.Edge>
  for +listed: Bool
  for h: {R.on_edge(l, all, listed, e) == True{} : Bool}
  {R.any_edge(List.append(&2, R.Edge, xs, e <> ys), l, all, listed) == True{} : Bool}
