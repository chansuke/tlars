//! The `MP.getMessage0` cases that compute their text. One function per code, named after
//! the code; `catalog.rs` (generated) refers to each by name. Each returns the template
//! *before* `%i%` replacement, exactly as the Java case appends it.

use crate::{MsgClass, RenderCx};

const CONFIG_FILE_ERROR: &str = "TLC found an error in the configuration file at line %1%\n";
/// `MP.NOT_APPLICABLE_VAL`.
const NOT_APPLICABLE: &str = "-1";
/// `TLAConstants.BACK_TO_STATE`.
const BACK_TO_STATE: &str = "Back to state";
/// `TLAConstants.STUTTERING`.
const STUTTERING: &str = " Stuttering";

pub(crate) fn system_error_reading_pool(p: &[&str], _: &RenderCx) -> String {
    if p.len() == 2 { "when reading pool file %2% (StatePoolReader.run):\n%1%" } else { "when reading the disk (StatePoolReader.run):\n%1%" }
        .into()
}

pub(crate) fn system_error_cleaning_pool(_: &[&str], cx: &RenderCx) -> String {
    match cx.class {
        MsgClass::Error => "Exception cleaning up an obsolete disk file.\n%1%",
        MsgClass::Warning => "Failed to clean up an obsolete disk file. Please manually delete %1% if free disk space is low.",
        _ => "",
    }
    .into()
}

pub(crate) fn tlc_state_not_completely_specified_next(p: &[&str], _: &RenderCx) -> String {
    match p.len() {
        3 => "Successor state is not completely specified by action %1% of the next-state relation. The following variable%2% not defined: %3%.\n",
        2 => "Successor state is not completely specified by the next-state action. The following variable%1% not defined: %2%.\n",
        _ => "Successor state is not completely specified by the next-state action.\n",
    }
    .into()
}

pub(crate) fn tlc_invariant_violated_level(p: &[&str], _: &RenderCx) -> String {
    let mut b = String::from("The invariant %1% is not a state predicate (one with no primes or temporal operators).");
    if p.len() > 1 {
        b.push_str(
            "\nNote that a bug can cause TLC to incorrectly report this error.\n\
             If you believe your TLA+ or PlusCal specification to be correct,\n\
             please check if this bug described in LevelNode.java starting at line 590ff affects you.",
        );
    }
    b
}

pub(crate) fn tlc_invariant_evaluation_failed(p: &[&str], _: &RenderCx) -> String {
    match p.len() {
        1 => "Evaluating invariant %1% failed.",
        2 => "Evaluating invariant %1% failed.\n%2%",
        _ => "",
    }
    .into()
}

pub(crate) fn tlc_action_property_evaluation_failed(p: &[&str], _: &RenderCx) -> String {
    match p.len() {
        1 => "Evaluating action property %1% failed.",
        2 => "Evaluating action property %1% failed.\n%2%",
        _ => "",
    }
    .into()
}

/// "Temporal properties A, B, and C were violated." with TLC's Oxford comma.
pub(crate) fn tlc_temporal_property_violated(p: &[&str], _: &RenderCx) -> String {
    match p.len() {
        0 => "Temporal properties were violated.\n".into(),
        1 => "Temporal property %1% was violated.\n".into(),
        n => {
            let mut b = String::from("Temporal properties ");
            for i in 1..=n {
                if i > 1 && i < n {
                    b.push_str(", ");
                } else if i == n {
                    b.push_str(if n == 2 { " and " } else { ", and " });
                }
                b.push_str(&format!("%{i}%"));
            }
            b.push_str(" were violated.\n");
            b
        }
    }
}

pub(crate) fn tlc_back_to_state(p: &[&str], cx: &RenderCx) -> String {
    match (cx.tool_mode, p.len()) {
        (true, 1) => format!("%1%: {BACK_TO_STATE}\n"),
        (true, 2) => format!("%1%: {BACK_TO_STATE}: %2%\n"),
        (false, 1) => format!("{BACK_TO_STATE} %1%\n"),
        (false, 2) => format!("{BACK_TO_STATE} %1%: %2%\n"),
        _ => String::new(),
    }
}

pub(crate) fn check_param_unrecognized(p: &[&str], _: &RenderCx) -> String {
    match p.len() {
        1 => "Unrecognized option: %1%",
        2 => "Unrecognized option in file %2%: %1%",
        _ => "",
    }
    .into()
}

pub(crate) fn tlc_live_cannot_handle_formula(p: &[&str], _: &RenderCx) -> String {
    if p.len() > 1 { "TLC cannot handle the temporal formula %1%:\n%2%" } else { "TLC cannot handle the temporal formula %1%" }.into()
}

pub(crate) fn tlc_distributed_server_running(_: &[&str], cx: &RenderCx) -> String {
    format!("TLC server at %1% is ready ({})", cx.now)
}
pub(crate) fn tlc_distributed_worker_registered(_: &[&str], cx: &RenderCx) -> String {
    format!("Registration for worker at %1% completed ({})", cx.now)
}
pub(crate) fn tlc_distributed_worker_deregistered(_: &[&str], cx: &RenderCx) -> String {
    format!("TLC worker %1% disconnected ({})", cx.now)
}
pub(crate) fn tlc_distributed_worker_stats(_: &[&str], cx: &RenderCx) -> String {
    format!("Worker: %1% Sent: %2% Rcvd: %3% CacheRatio: %4% ({})", cx.now)
}
pub(crate) fn tlc_distributed_server_not_running(_: &[&str], cx: &RenderCx) -> String {
    format!("TLCServer is gone due to %1%, exiting worker... ({})", cx.now)
}
pub(crate) fn tlc_distributed_server_finished(_: &[&str], cx: &RenderCx) -> String {
    format!("TLCServer has finished, exiting worker... ({})", cx.now)
}
pub(crate) fn tlc_distributed_vm_version(_: &[&str], cx: &RenderCx) -> String {
    format!("VM does not allow to get the UnicastRef port.\nWorker will be identified with port 0 in output ({})", cx.now)
}
pub(crate) fn tlc_distributed_worker_lost(_: &[&str], cx: &RenderCx) -> String {
    format!("TLC worker connection lost %1% ({})", cx.now)
}
pub(crate) fn tlc_distributed_exceed_blocksize(_: &[&str], cx: &RenderCx) -> String {
    format!("Trying to limit max block size (to recover from transport failure): %1% ({})", cx.now)
}
pub(crate) fn tlc_distributed_server_fpset_registered(_: &[&str], cx: &RenderCx) -> String {
    format!("%1% out of %2% FPSet server(s) registered ({})", cx.now)
}
pub(crate) fn tlc_distributed_server_fpset_waiting(_: &[&str], cx: &RenderCx) -> String {
    format!("Waiting for %1% FPSet server(s) to register ({})", cx.now)
}

pub(crate) fn tlc_starting(_: &[&str], cx: &RenderCx) -> String {
    format!("Starting... ({})", cx.now)
}

pub(crate) fn tlc_finished(_: &[&str], cx: &RenderCx) -> String {
    format!("Finished in %1% at ({})", cx.now)
}

/// Parameter 14 (`%14%`) is the pid; an empty pid prints `%14%` itself, which then expands
/// to the empty string.
fn pid_part(p: &[&str], index: usize, placeholder: usize) -> String {
    if p.get(index).is_some_and(|s| !s.is_empty()) { format!(" [pid: %{placeholder}%]") } else { format!("%{placeholder}%") }
}

pub(crate) fn tlc_mode_mc(p: &[&str], _: &RenderCx) -> String {
    format!(
        "Running breadth-first search Model-Checking with fp %13% and seed %12% with %1% worker%2% on %3% cores with %10%MB heap and %11%MB offheap memory{} (%4% %5% %6%, %7% %8% %9%, %15%, %16%).",
        pid_part(p, 13, 14)
    )
}

pub(crate) fn tlc_mode_mc_dfs(p: &[&str], _: &RenderCx) -> String {
    format!(
        "Running depth-first search Model-Checking with fp %13% and seed %12% with %1% worker%2% on %3% cores with %10%MB heap and %11%MB offheap memory{} (%4% %5% %6%, %7% %8% %9%).",
        pid_part(p, 13, 14)
    )
}

pub(crate) fn tlc_mode_simu(p: &[&str], _: &RenderCx) -> String {
    format!(
        "Running %14% Simulation with seed %1% with %2% worker%3% on %4% cores with %11%MB heap and %12%MB offheap memory{} (%5% %6% %7%, %8% %9% %10%).",
        pid_part(p, 12, 13)
    )
}

pub(crate) fn tlc_init_generated1(_: &[&str], cx: &RenderCx) -> String {
    format!("Finished computing initial states: %1% distinct state%2% generated at {}.", cx.now)
}

pub(crate) fn tlc_init_generated2(_: &[&str], cx: &RenderCx) -> String {
    format!("Finished computing initial states: %1% state%2% generated, with %3% of them distinct at {}.", cx.now)
}

pub(crate) fn tlc_init_generated3(_: &[&str], cx: &RenderCx) -> String {
    format!(
        "Finished computing initial states: %1% states generated.\nBecause TLC recovers from a previous checkpoint, only %2% of them require further exploration at {}.",
        cx.now
    )
}

pub(crate) fn tlc_checking_temporal_props(_: &[&str], cx: &RenderCx) -> String {
    format!("Checking %3%temporal properties for the %1% state space with %2% total distinct states at ({})", cx.now)
}

pub(crate) fn tlc_checking_temporal_props_end(_: &[&str], cx: &RenderCx) -> String {
    format!("Finished checking temporal properties in %1% at {}", cx.now)
}

pub(crate) fn tlc_success(p: &[&str], _: &RenderCx) -> String {
    let mut b = String::from(
        "Model checking completed. No error has been found.\n  Estimates of the probability that TLC did not check all reachable states\n",
    );
    if p.len() == 1 {
        b.push_str("  because two distinct states had the same fingerprint:\n  calculated (optimistic):  %1%");
    } else {
        b.push_str(
            "  because two distinct states had the same fingerprint:\n  calculated (optimistic):  %1%\n  based on the actual fingerprints:  %2%",
        );
    }
    b
}

pub(crate) fn tlc_checkpoint_end(_: &[&str], cx: &RenderCx) -> String {
    format!("Checkpointing completed at ({})", cx.now)
}

pub(crate) fn tlc_progress_stats(p: &[&str], cx: &RenderCx) -> String {
    match p.len() {
        4 => format!(
            "Progress(%1%) at {}: %2% states generated, %3% distinct states found, %4% states left on queue.",
            cx.now
        ),
        6 => format!(
            "Progress(%1%) at {}: %2% states generated (%5% s/min), %3% distinct states found (%6% ds/min), %4% states left on queue.",
            cx.now
        ),
        _ => String::new(),
    }
}

pub(crate) fn tlc_progress_stats_dfid(_: &[&str], cx: &RenderCx) -> String {
    if cx.tool_mode {
        format!(
            "Progress({NOT_APPLICABLE}) at {}: %1% states generated, %2% distinct states found, {NOT_APPLICABLE} states left on queue.",
            cx.now
        )
    } else {
        "Progress: %1% states generated, %2% distinct states found.".into()
    }
}

pub(crate) fn tlc_progress_simu(_: &[&str], cx: &RenderCx) -> String {
    if cx.tool_mode {
        format!(
            "Progress(%2%) at {}: %1% states generated, {NOT_APPLICABLE} distinct states found, {NOT_APPLICABLE} states left on queue.",
            cx.now
        )
    } else {
        "Progress: %1% states checked, %2% traces generated (trace length: mean=%3%, var(x)=%4%, sd=%5%)".into()
    }
}

pub(crate) fn tlc_coverage_start(_: &[&str], cx: &RenderCx) -> String {
    format!(
        "The coverage statistics at {} (see https://explain.tlapl.us/module-coverage-statistics for how to interpret the following statistics).",
        cx.now
    )
}

pub(crate) fn tlc_config_id_requires_no_arg(p: &[&str], _: &RenderCx) -> String {
    match p.len() {
        1 => "TLC requires %1% not to take any argument.",
        2 => "TLC requires %1% not to take any argument, but one was given: %2%",
        _ => "",
    }
    .into()
}

pub(crate) fn tlc_state_print2(_: &[&str], cx: &RenderCx) -> String {
    if cx.debug { "%1%: %2%\n%3%fp: %4%\n" } else { "%1%: %2%\n%3%" }.into()
}

pub(crate) fn tlc_state_print3(_: &[&str], _: &RenderCx) -> String {
    format!("%1%:{STUTTERING}")
}

pub(crate) fn cfg_missing_id(_: &[&str], _: &RenderCx) -> String {
    format!("{CONFIG_FILE_ERROR}The keyword %2% was not followed by an identifier.")
}

pub(crate) fn cfg_twice_keyword(_: &[&str], _: &RenderCx) -> String {
    format!("{CONFIG_FILE_ERROR}The keyword %2% appeared twice.")
}

pub(crate) fn cfg_expect_id(_: &[&str], _: &RenderCx) -> String {
    format!("{CONFIG_FILE_ERROR}Expected an identifier after %2%.")
}

pub(crate) fn cfg_general(_: &[&str], _: &RenderCx) -> String {
    CONFIG_FILE_ERROR.into()
}

pub(crate) fn cfg_expected_symbol(_: &[&str], _: &RenderCx) -> String {
    format!("{CONFIG_FILE_ERROR}It was expecting %2%, but did not find it.")
}

/// Adapts to the number of parameters: `%1%%2%...`.
pub(crate) fn general(p: &[&str], _: &RenderCx) -> String {
    (1..=p.len()).map(|i| format!("%{i}%")).collect()
}
