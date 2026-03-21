//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::BinaryHeap;
use alloc::sync::Arc;
use core::cmp::Ordering;
use lazy_static::*;

#[derive(Clone)]
struct StrideTask {
    stride: u64,
    task: Arc<TaskControlBlock>,
}

impl StrideTask {
    fn new(task: Arc<TaskControlBlock>) -> Self {
        Self {
            stride: task.get_stride(),
            task,
        }
    }
}

impl PartialEq for StrideTask {
    fn eq(&self, other: &Self) -> bool {
        self.stride == other.stride
    }
}

impl Eq for StrideTask {}

impl PartialOrd for StrideTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StrideTask {
    fn cmp(&self, other: &Self) -> Ordering {
        let diff = self.stride.wrapping_sub(other.stride) as i64;
        if diff == 0 {
            Ordering::Equal
        } else if diff < 0 {
            Ordering::Greater
        } else {
            Ordering::Less
        }
    }
}
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: BinaryHeap<StrideTask>,
}

/// A simple stride scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: BinaryHeap::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push(StrideTask::new(task));
    }
    /// Take a process out of the ready queue
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        self.ready_queue.pop().map(|t| t.task)
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
