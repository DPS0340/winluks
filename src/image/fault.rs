//! Deterministic syscall-boundary faults. Compiled only in unit-test builds.
use std::{collections::VecDeque, io, sync::Mutex};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Read,
    Write,
    Sync,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Action {
    Limit(usize),
    Fail(io::ErrorKind),
    Panic,
}
#[derive(Default)]
pub(crate) struct Injector {
    plan: Mutex<VecDeque<(Operation, Action)>>,
    events: Mutex<Vec<Operation>>,
}
impl Injector {
    pub(crate) fn set(&self, steps: &[(Operation, Action)]) {
        *self.plan.lock().unwrap() = steps.iter().copied().collect();
        self.events.lock().unwrap().clear();
    }
    pub(crate) fn events(&self) -> Vec<Operation> {
        self.events.lock().unwrap().clone()
    }
    pub(crate) fn before(&self, op: Operation, size: usize) -> io::Result<usize> {
        self.events.lock().unwrap().push(op);
        let next = { self.plan.lock().unwrap().pop_front() };
        match next {
            None => Ok(size),
            Some((expected, action)) => {
                assert_eq!(expected, op, "unexpected syscall order");
                match action {
                    Action::Limit(n) => Ok(size.min(n)),
                    Action::Fail(kind) => Err(io::Error::from(kind)),
                    Action::Panic => panic!("synthetic backend panic"),
                }
            }
        }
    }
}
