use std::ops::{Deref, Mul};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::{thread, time};

use log::{debug, error, info, trace, warn};
use rdev::{simulate, EventType};

use crate::args::Args;
use crate::binding::Binding;
use crate::compare_angles::compare_angles_with_offset;
use crate::config::Config;
use crate::event::{edges_are_equals, modifiers_are_equals, ClickEvent, PressState};

const DIFF_MAX: f64 = 0.6;
const DIFF_MIN_WITH_SECOND: f64 = 0.05;
const DIFF_MAX_PRINT: f64 = 300.0;
const SHAPE_MIN_SIZE: usize = 10;

/// filter the binding[] of config : keep bindings that have the same button, edges and modifiers
pub fn find_candidates<'a>(config: &'a Config, event: &ClickEvent) -> Vec<&'a Binding> {
    let shape_button = &config.shape_button;
    config
        .bindings
        .iter()
        .filter(|binding| {
            (binding.event.shape.is_empty()
                || shape_button != &binding.event.button
                || event.event_type != PressState::Press)
                && binding.event.button == event.button
                && (binding.event.event_type == event.event_type
                    || binding.event.event_type == PressState::Click)
                && edges_are_equals(&binding.event.edges, &event.edges)
                && modifiers_are_equals(&binding.event.modifiers, &event.modifiers)
        })
        .collect::<Vec<&Binding>>()
}

pub fn find_candidates_with_shape_with_offset<'a>(
    candidates: &'a [&Binding],
    event: &ClickEvent,
) -> Vec<(&'a &'a Binding, f64)> {
    debug!(
        "angles: {}",
        event
            .shape
            .iter()
            .map(|a| format!("{:.2}, ", a))
            .collect::<String>()
    );
    let start = Instant::now();
    let mut candidates_with_shape = candidates
        .iter()
        .filter(|b| !b.event.shape.is_empty())
        .map(|b| (b, compare_angles_with_offset(&event.shape, &b.event.shape)))
        .filter(|(_, diff)| *diff < DIFF_MAX_PRINT)
        .collect::<Vec<_>>();
    candidates_with_shape.sort_by(|(_, d1), (_, d2)| d1.partial_cmp(d2).unwrap());
    debug!(
        "find_candidates_with_shape_with_offset duration : {:?}",
        start.elapsed()
    );
    candidates_with_shape
}

pub fn find_the_chosen_one_among_the_candidates_with_shape<'a>(
    candidates: &'a [&Binding],
    event: &ClickEvent,
) -> Option<&'a Binding> {
    if event.shape.len() > SHAPE_MIN_SIZE {
        let candidates_with_shape = find_candidates_with_shape_with_offset(candidates, event);
        if !candidates_with_shape.is_empty() {
            debug!("shape candidates=");
            candidates_with_shape
                .iter()
                .take(5)
                .for_each(|(binding, diff)| {
                    let pc = f64::max(0., 100.0 - diff.powi(2).mul(100.));
                    debug!(
                        "   {:05.2} %    {diff:.2} : {}    {:?}",
                        pc, binding.comment, binding.cmd
                    )
                });

            // FIXME
            if !candidates_with_shape.is_empty()
                && candidates_with_shape.first().unwrap().1 < DIFF_MAX
                && (candidates_with_shape.len() > 1
                    && candidates_with_shape.get(1).unwrap().1
                        - candidates_with_shape.get(0).unwrap().1
                        > DIFF_MIN_WITH_SECOND)
            {
                return Some(candidates_with_shape.first().unwrap().0);
            }
        }
    }
    None
}

pub fn find_the_chosen_one_among_the_candidates_without_shape<'a>(
    candidates: &'a [&Binding],
    event: &ClickEvent,
) -> Option<&'a Binding> {
    let candidates_without_shape = candidates
        .iter()
        .filter(|b| b.event.shape.is_empty())
        .collect::<Vec<_>>();

    match candidates_without_shape.len() {
        1 => {
            let binding = candidates_without_shape.first().unwrap();
            debug!("Binding without shape found : {:?}", binding);
            return Some(binding);
        }
        0 => {}
        _ => {
            warn!(
                "WARNING, several candidate ! ev = {:?} candidates = {:?}",
                event, candidates_without_shape
            );
        }
    }
    None
}

pub fn find_the_chosen_one_among_the_candidates<'a>(
    candidates: &'a [&Binding],
    event: &ClickEvent,
) -> Option<&'a Binding> {
    find_the_chosen_one_among_the_candidates_with_shape(candidates, event)
        .or_else(|| find_the_chosen_one_among_the_candidates_without_shape(candidates, event))
}

pub fn trace_event(_config: Arc<Mutex<Config>>, event: ClickEvent, _args: Arc<Args>) -> bool {
    println!("event={:?}", event);
    true
}

/// Execute the command of the event if the corresponding binding is found.
/// return false if the event must not be propagated
pub fn process_event(config: Arc<Mutex<Config>>, event: ClickEvent, _args: Arc<Args>) -> bool {
    let mut propagate = true;
    let start = Instant::now();
    let config_lock = config.lock().unwrap();
    let config = config_lock.deref();
    let candidates = find_candidates(config, &event);
    trace!("event={:?}", event);
    trace!("candidates={:?}", candidates);

    if !candidates.is_empty() {
        if let Some(binding) = find_the_chosen_one_among_the_candidates(&candidates, &event) {
            propagate = false;
            if !(event.event_type == PressState::Release
                && binding.event.event_type == PressState::Click
                && binding.event.shape.is_empty())
            {
                process_cmd(binding.cmd.clone());
            }
        } else if event.event_type == PressState::Release && event.button == config.shape_button {
            propagate = false;
            let rdev_btn = config.shape_button.to_rdev_event();

            trace!("simulate");
            simulate(&EventType::ButtonPress(rdev_btn))
                .map_err(|err| error!("simulate err: {:?}", err))
                .ok();
            thread::sleep(time::Duration::from_millis(10));
            simulate(&EventType::ButtonRelease(rdev_btn))
                .map_err(|err| error!("simulate err: {:?}", err))
                .ok();
        }
    }
    trace!("propagate = {propagate}");
    if !propagate {
        debug!("Process event duration : {:?}", start.elapsed());
    }
    propagate
}

fn process_cmd(cmd: Vec<String>) {
    thread::spawn(move || {
        info!("     → cmd {:?}", cmd);
        Command::new(&cmd[0])
            .env("RUST_LOG", "")
            .args(&cmd[1..])
            .status()
            .map_err(|err| error!("Command err: {:?}", err))
            .ok();
        debug!("end of process_cmd thread");
    });
}