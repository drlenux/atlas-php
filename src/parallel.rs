//! Scoped workers borrow existing result slots; no unbounded job or result queue.
use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
};

pub(crate) fn try_for_each_mut<T, S, I, F>(
    items: &mut [T],
    workers: usize,
    initialize: I,
    action: F,
) -> Result<(), String>
where
    T: Send,
    I: Fn() -> S + Sync,
    F: Fn(&mut S, usize, &mut T) -> Result<(), String> + Sync,
{
    if !(1..=32).contains(&workers) {
        return Err("indexing workers must be in 1..32".into());
    }
    let workers = workers.min(items.len());
    if workers == 0 {
        return Ok(());
    }
    let pending = Mutex::new(items.iter_mut().enumerate());
    let stopped = AtomicBool::new(false);
    let failure = Mutex::new(None);
    let fail = |error: String| {
        stopped.store(true, Ordering::Release);
        failure.lock().unwrap().get_or_insert(error);
    };
    let run = || {
        let result = catch_unwind(AssertUnwindSafe(|| {
            let mut state = initialize();
            loop {
                let task = {
                    let mut pending = pending.lock().unwrap();
                    if stopped.load(Ordering::Acquire) {
                        return Ok(());
                    }
                    pending.next()
                };
                let Some((position, item)) = task else {
                    return Ok(());
                };
                action(&mut state, position, item)?;
            }
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => fail(error),
            Err(_) => fail("indexing worker panicked; snapshot not published".into()),
        }
    };
    if workers == 1 {
        run();
    } else {
        thread::scope(|scope| {
            for number in 0..workers {
                if let Err(error) = thread::Builder::new()
                    .name(format!("atlas-index-{number}"))
                    .spawn_scoped(scope, run)
                {
                    fail(format!("cannot start indexing worker: {error}"));
                    break;
                }
            }
        });
    }
    failure.into_inner().unwrap().map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Condvar, Mutex,
    };
    use std::time::Duration;

    #[test]
    fn workers_overlap_with_a_fixed_limit_and_keep_result_slots_in_order() {
        let mut slots = vec![0; 24];
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let gate = (Mutex::new(0), Condvar::new());
        try_for_each_mut(
            &mut slots,
            4,
            || (),
            |_, position, slot| {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                if position < 4 {
                    let mut arrived = gate.0.lock().unwrap();
                    *arrived += 1;
                    gate.1.notify_all();
                    let (arrived, _) = gate
                        .1
                        .wait_timeout_while(arrived, Duration::from_secs(2), |n| *n < 4)
                        .unwrap();
                    if *arrived < 4 {
                        return Err("jobs did not overlap".into());
                    }
                }
                *slot = position * 2;
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(peak.load(Ordering::SeqCst), 4);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(slots, (0..24).map(|n| n * 2).collect::<Vec<_>>());
    }

    #[test]
    fn errors_stop_new_work_and_panics_return_an_error_after_join() {
        let mut slots = vec![0; 20];
        let error = try_for_each_mut(
            &mut slots,
            1,
            || (),
            |_, position, slot| {
                if position == 2 {
                    return Err("analysis failed".into());
                }
                *slot = 1;
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error, "analysis failed");
        assert_eq!(&slots[..4], &[1, 1, 0, 0]);
        assert!(try_for_each_mut(
            &mut slots,
            4,
            || (),
            |_, position, _| {
                assert_ne!(position, 0, "fixture panic");
                Ok(())
            }
        )
        .unwrap_err()
        .contains("panicked"));
        let initialized = AtomicUsize::new(0);
        try_for_each_mut(
            &mut [] as &mut [usize],
            4,
            || {
                initialized.fetch_add(1, Ordering::SeqCst);
            },
            |_, _, _| Ok(()),
        )
        .unwrap();
        assert_eq!(initialized.load(Ordering::SeqCst), 0);
    }
}
