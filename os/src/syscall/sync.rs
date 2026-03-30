use crate::sync::{Condvar, Mutex, MutexBlocking, MutexSpin, Semaphore};
use crate::task::{block_current_and_run_next, current_process, current_task};
use crate::timer::{add_timer, get_time_ms};
use alloc::{sync::Arc, vec, vec::Vec};

const DEADLOCK_ERR: isize = -0xDEAD;

fn current_tid() -> usize {
    current_task()
        .unwrap()
        .inner_exclusive_access()
        .res
        .as_ref()
        .unwrap()
        .tid
}

fn ensure_deadlock_state(process_inner: &mut crate::task::ProcessControlBlockInner) {
    let n = process_inner.tasks.len();
    if process_inner.mutex_waiting.len() < n {
        process_inner.mutex_waiting.resize(n, None);
    }
    if process_inner.sem_waiting.len() < n {
        process_inner.sem_waiting.resize(n, None);
    }
    let m_sem = process_inner.semaphore_list.len();
    if process_inner.sem_alloc.len() < n {
        process_inner.sem_alloc.resize_with(n, || vec![0; m_sem]);
    }
    for row in process_inner.sem_alloc.iter_mut() {
        if row.len() < m_sem {
            row.resize(m_sem, 0);
        }
    }
    if process_inner.sem_total.len() < m_sem {
        process_inner.sem_total.resize(m_sem, 0);
    }
    if process_inner.mutex_owner.len() < process_inner.mutex_list.len() {
        process_inner
            .mutex_owner
            .resize(process_inner.mutex_list.len(), None);
    }
}

fn safe_check(
    available: &[usize],
    allocation: &[Vec<usize>],
    need: &[Vec<usize>],
    finish: &mut [bool],
) -> bool {
    let m = available.len();
    let n = finish.len();
    let mut work = available.to_vec();
    loop {
        let mut found = None;
        for i in 0..n {
            if finish[i] {
                continue;
            }
            let mut ok = true;
            for j in 0..m {
                if need[i][j] > work[j] {
                    ok = false;
                    break;
                }
            }
            if ok {
                found = Some(i);
                break;
            }
        }
        let Some(i) = found else {
            break;
        };
        for j in 0..m {
            work[j] += allocation[i][j];
        }
        finish[i] = true;
    }
    finish.iter().all(|x| *x)
}

fn mutex_safe(process_inner: &crate::task::ProcessControlBlockInner) -> bool {
    let n = process_inner.tasks.len();
    let m = process_inner.mutex_list.len();
    let mut available = vec![1usize; m];
    let mut allocation = vec![vec![0usize; m]; n];
    let mut need = vec![vec![0usize; m]; n];
    let mut finish = vec![false; n];
    for i in 0..n {
        finish[i] = process_inner.tasks[i].is_none();
        if let Some(mid) = process_inner.mutex_waiting.get(i).and_then(|x| *x) {
            if mid < m {
                need[i][mid] = 1;
            }
        }
    }
    for mid in 0..m {
        if let Some(owner) = process_inner.mutex_owner.get(mid).and_then(|x| *x) {
            available[mid] = 0;
            if owner < n {
                allocation[owner][mid] = 1;
            }
        }
    }
    safe_check(&available, &allocation, &need, &mut finish)
}

fn sem_safe(process_inner: &crate::task::ProcessControlBlockInner) -> bool {
    let n = process_inner.tasks.len();
    let m = process_inner.semaphore_list.len();
    let mut available = vec![0usize; m];
    let mut allocation = vec![vec![0usize; m]; n];
    let mut need = vec![vec![0usize; m]; n];
    let mut finish = vec![false; n];
    for i in 0..n {
        finish[i] = process_inner.tasks[i].is_none();
        if let Some(sid) = process_inner.sem_waiting.get(i).and_then(|x| *x) {
            if sid < m {
                need[i][sid] = 1;
            }
        }
        if i < process_inner.sem_alloc.len() {
            for j in 0..m {
                allocation[i][j] = process_inner.sem_alloc[i].get(j).copied().unwrap_or(0);
            }
        }
    }
    for j in 0..m {
        let mut used = 0usize;
        for i in 0..n {
            used += allocation[i][j];
        }
        let total = process_inner.sem_total.get(j).copied().unwrap_or(0);
        available[j] = total.saturating_sub(used);
    }
    safe_check(&available, &allocation, &need, &mut finish)
}
/// sleep syscall
pub fn sys_sleep(ms: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_sleep",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let expire_ms = get_time_ms() + ms;
    let task = current_task().unwrap();
    add_timer(expire_ms, task);
    block_current_and_run_next();
    0
}
/// mutex create syscall
pub fn sys_mutex_create(blocking: bool) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mutex: Option<Arc<dyn Mutex>> = if !blocking {
        Some(Arc::new(MutexSpin::new()))
    } else {
        Some(Arc::new(MutexBlocking::new()))
    };
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    if let Some(id) = process_inner
        .mutex_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.mutex_list[id] = mutex;
        if process_inner.mutex_owner.len() <= id {
            process_inner.mutex_owner.resize(id + 1, None);
        }
        id as isize
    } else {
        process_inner.mutex_list.push(mutex);
        process_inner.mutex_owner.push(None);
        process_inner.mutex_list.len() as isize - 1
    }
}
/// mutex lock syscall
pub fn sys_mutex_lock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_lock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    if mutex_id >= process_inner.mutex_list.len() {
        return -1;
    }
    if process_inner.deadlock_detect_enabled {
        let locked = process_inner
            .mutex_owner
            .get(mutex_id)
            .and_then(|x| *x)
            .is_some();
        if locked {
            process_inner.mutex_waiting[tid] = Some(mutex_id);
            if !mutex_safe(&process_inner) {
                process_inner.mutex_waiting[tid] = None;
                return DEADLOCK_ERR;
            }
        }
    }
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.lock();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    process_inner.mutex_waiting[tid] = None;
    process_inner.mutex_owner[mutex_id] = Some(tid);
    0
}
/// mutex unlock syscall
pub fn sys_mutex_unlock(mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_mutex_unlock",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    if mutex_id >= process_inner.mutex_list.len() {
        return -1;
    }
    if process_inner.mutex_owner.get(mutex_id).and_then(|x| *x) == Some(tid) {
        process_inner.mutex_owner[mutex_id] = None;
    }
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    drop(process);
    mutex.unlock();
    0
}
/// semaphore create syscall
pub fn sys_semaphore_create(res_count: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    let id = if let Some(id) = process_inner
        .semaphore_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.semaphore_list[id] = Some(Arc::new(Semaphore::new(res_count)));
        if process_inner.sem_total.len() <= id {
            process_inner.sem_total.resize(id + 1, 0);
        }
        process_inner.sem_total[id] = res_count;
        id
    } else {
        process_inner
            .semaphore_list
            .push(Some(Arc::new(Semaphore::new(res_count))));
        process_inner.sem_total.push(res_count);
        process_inner.semaphore_list.len() - 1
    };
    id as isize
}
/// semaphore up syscall
pub fn sys_semaphore_up(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_up",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    if sem_id >= process_inner.semaphore_list.len() {
        return -1;
    }
    if tid < process_inner.sem_alloc.len() && sem_id < process_inner.sem_alloc[tid].len() {
        if process_inner.sem_alloc[tid][sem_id] > 0 {
            process_inner.sem_alloc[tid][sem_id] -= 1;
        }
    }
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    drop(process_inner);
    sem.up();
    0
}
/// semaphore down syscall
pub fn sys_semaphore_down(sem_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_semaphore_down",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let tid = current_tid();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    if sem_id >= process_inner.semaphore_list.len() {
        return -1;
    }
    let sem = Arc::clone(process_inner.semaphore_list[sem_id].as_ref().unwrap());
    let would_block = sem.inner.exclusive_access().count <= 0;
    if process_inner.deadlock_detect_enabled && would_block {
        process_inner.sem_waiting[tid] = Some(sem_id);
        if !sem_safe(&process_inner) {
            process_inner.sem_waiting[tid] = None;
            return DEADLOCK_ERR;
        }
    }
    drop(process_inner);
    drop(process);
    sem.down();
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    process_inner.sem_waiting[tid] = None;
    process_inner.sem_alloc[tid][sem_id] += 1;
    0
}
/// condvar create syscall
pub fn sys_condvar_create() -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_create",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    let id = if let Some(id) = process_inner
        .condvar_list
        .iter()
        .enumerate()
        .find(|(_, item)| item.is_none())
        .map(|(id, _)| id)
    {
        process_inner.condvar_list[id] = Some(Arc::new(Condvar::new()));
        id
    } else {
        process_inner
            .condvar_list
            .push(Some(Arc::new(Condvar::new())));
        process_inner.condvar_list.len() - 1
    };
    id as isize
}
/// condvar signal syscall
pub fn sys_condvar_signal(condvar_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_signal",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    drop(process_inner);
    condvar.signal();
    0
}
/// condvar wait syscall
pub fn sys_condvar_wait(condvar_id: usize, mutex_id: usize) -> isize {
    trace!(
        "kernel:pid[{}] tid[{}] sys_condvar_wait",
        current_task().unwrap().process.upgrade().unwrap().getpid(),
        current_task()
            .unwrap()
            .inner_exclusive_access()
            .res
            .as_ref()
            .unwrap()
            .tid
    );
    let process = current_process();
    let process_inner = process.inner_exclusive_access();
    let condvar = Arc::clone(process_inner.condvar_list[condvar_id].as_ref().unwrap());
    let mutex = Arc::clone(process_inner.mutex_list[mutex_id].as_ref().unwrap());
    drop(process_inner);
    condvar.wait(mutex);
    0
}
/// enable deadlock detection syscall
///
/// YOUR JOB: Implement deadlock detection, but might not all in this syscall
pub fn sys_enable_deadlock_detect(_enabled: usize) -> isize {
    trace!("kernel: sys_enable_deadlock_detect");
    if _enabled != 0 && _enabled != 1 {
        return -1;
    }
    let process = current_process();
    let mut process_inner = process.inner_exclusive_access();
    ensure_deadlock_state(&mut process_inner);
    process_inner.deadlock_detect_enabled = _enabled == 1;
    0
}
