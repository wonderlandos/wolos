# SPDX-License-Identifier: GPL-2.0-or-later
# Evidence for LAWS.bend. Run with `bend PROOF.bend`. Most proofs are the checker running on a
# small graph, so they establish behaviour on that graph; the `for` laws hold for every input.
import Base
import ./LAWS.bend as Laws
import ./rules.bend as R

def Laws.fetch_execute_unverified_found():
  {==}

def Laws.fetch_execute_unverified_clean():
  {==}

def Laws.a_check_after_the_run_is_no_check():
  {==}

def Laws.declared_vcs_tracking_is_not_a_stray_download():
  {==}

def Laws.decode_execute_found():
  {==}

def Laws.decode_execute_clean():
  {==}

def Laws.unresolved_dynamic_found():
  {==}

def Laws.unresolved_dynamic_clean():
  {==}

def Laws.a_known_path_into_dynamic_text_is_the_finding():
  {==}

def Laws.write_outside_package_found():
  {==}

def Laws.write_outside_package_clean():
  {==}

def Laws.a_scriptlet_is_judged_by_its_own_law():
  {==}

def Laws.upstream_build_files_stay_in_the_tree_they_describe():
  {==}

def Laws.read_credentials_found():
  {==}

def Laws.read_credentials_clean():
  {==}

def Laws.credential_to_network_found():
  {==}

def Laws.credential_to_network_clean():
  {==}

def Laws.scriptlet_undeclared_file_found():
  {==}

def Laws.scriptlet_undeclared_file_clean():
  {==}

def Laws.a_mode_change_waits_for_the_real_file_list():
  {==}

def Laws.source_off_upstream_found():
  {==}

def Laws.source_off_upstream_clean():
  {==}

def Laws.undeclared_persistence_found():
  {==}

def Laws.undeclared_persistence_clean():
  {==}

def Laws.a_mode_change_starts_nothing():
  {==}

def Laws.postinstall_network_found():
  {==}

def Laws.postinstall_network_clean():
  {==}

def Laws.bin_checksum_upstream_found():
  {==}

def Laws.bin_checksum_differs_found():
  {==}

def Laws.bin_checksum_upstream_clean():
  {==}

def Laws.unexplained_new_edge_found():
  {==}

def Laws.unexplained_new_edge_clean():
  {==}

def Laws.observed_mismatch_found():
  {==}

def Laws.observed_mismatch_clean():
  {==}

def Laws.incomplete_evidence_found():
  {==}

def Laws.incomplete_evidence_clean():
  {==}

def Laws.nothing_broken_says_exactly_that():
  {==}

def Laws.any_broken_law_asks_for_review(l, ls):
  {==}

def Laws.a_gap_is_never_lost(ts, ns):
  {==}

def Laws.reading_a_secret_is_always_found(n, ts, d, s):
  {==}

def Laws.an_unchecked_network_run_is_always_found(all, n, d, seq, h):
  %Equal.sym(Bool, R.checked(all, n, seq), False{}, h) : {Bool.and(Bool.not(Bool.or(_, False{})), True{}) == True{} : Bool}
  {==}

def Laws.true_absorbs_or(b):
  match b:
    case False{}:
      {==}
    case True{}:
      {==}

def Laws.a_broken_edge_is_found_anywhere_in_the_list(xs, e, ys, l, all, listed, h):
  match xs:
    case Nil{}:
      %Equal.sym(Bool, R.on_edge(l, all, listed, e), True{}, h) : {Bool.or(_, R.any_edge(ys, l, all, listed)) == True{} : Bool}
      {==}
    case Con{x, r}:
      %Equal.sym(Bool, R.any_edge(List.append(&2, R.Edge, r, e <> ys), l, all, listed), True{}, Laws.a_broken_edge_is_found_anywhere_in_the_list(r, e, ys, l, all, listed, h)) : {Bool.or(R.on_edge(l, all, listed, x), _) == True{} : Bool}
      Laws.true_absorbs_or(R.on_edge(l, all, listed, x))
