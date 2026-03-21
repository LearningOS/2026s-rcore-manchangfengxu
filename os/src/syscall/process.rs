//! Process management syscalls
const VA_WIDTH_SV39: usize = 39;
use crate::config::PAGE_SIZE;
use crate::{
    mm::{find_pte, translated_byte_buffer},
    task::{current_user_token, TASK_MANAGER},
    timer::get_time_us,
};
use crate::{
    mm::{MapPermission, VirtAddr},
    syscall::CALL_NUMBERS,
    task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next},
};
use core::mem::size_of;
#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    if ts as usize >= (1 << VA_WIDTH_SV39) {
        println!("_id {} over", ts as usize);
        return -1;
    }
    let us = get_time_us();
    let tv = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };

    let token = current_user_token();
    let mut dst = translated_byte_buffer(token, ts as *const u8, size_of::<TimeVal>());

    let src = unsafe {
        core::slice::from_raw_parts((&tv as *const TimeVal) as *const u8, size_of::<TimeVal>())
    };

    let mut written = 0usize;
    for chunk in dst.iter_mut() {
        let n = chunk.len().min(src.len() - written);
        chunk[..n].copy_from_slice(&src[written..written + n]);
        written += n;
        if written == src.len() {
            break;
        }
    }
    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(_trace_request: usize, _id: usize, _data: usize) -> isize {
    trace!("kernel: sys_trace");
    if _trace_request == 2 {
        let task_id = TASK_MANAGER.current_task_id();
        return unsafe { CALL_NUMBERS[task_id][_id] as isize };
    }
    if _id as usize >= (1 << VA_WIDTH_SV39) {
        println!("_id {} over", _id as usize);
        return -1;
    }
    let ptr = _id as *const u8;
    let token = current_user_token();
    let pte = match find_pte(token, ptr) {
        Some(pte) => pte,
        None => return -1,
    };

    match _trace_request {
        0 => {
            if !pte.readable() {
                return -1;
            }
            translated_byte_buffer(token, ptr, 1)[0][0] as isize
        }
        1 => {
            if !pte.writable() {
                return -1;
            }
            let mut buf = translated_byte_buffer(token, ptr, 1);
            buf[0][0] = _data as u8;
            0
        }
        _ => -1,
    }
}

// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!("kernel: sys_mmap NOT IMPLEMENTED YET!");
    if ((_port >> 3) > 0) || ((_port & 7) == 0) {
        println!("port error");
        return -1;
    }

    if _start % PAGE_SIZE != 0 {
        return -1;
    }
    if _start as usize >= (1 << VA_WIDTH_SV39) {
        println!("_start {} over", _start as usize);
        return -1;
    }
    if (_port & !0x7) != 0 {
        return -1;
    }
    if (_port & 0x7) == 0 {
        return -1;
    }
    if _len == 0 {
        return 0;
    }

    let end = match _start.checked_add(_len) {
        Some(end) => end,
        None => return -1,
    };
    let start_vpn = VirtAddr::from(_start).floor();
    let end_vpn = VirtAddr::from(end).ceil();

    let mut task = TASK_MANAGER.current_task();
    for vpn in start_vpn.0..end_vpn.0 {
        if let Some(pte) = task.memory_set.translate(crate::mm::VirtPageNum(vpn)) {
            if pte.is_valid() {
                return -1;
            }
        }
    }

    let permission = MapPermission::from_bits(((_port & 0b111 | 0b1000) << 1)  as u8).unwrap();
    task.memory_set.insert_framed_area_no_panic(
        VirtAddr::from(_start),
        VirtAddr::from(end),
        permission,
    )
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel: sys_munmap NOT IMPLEMENTED YET!");
    if _start as usize >= (1 << VA_WIDTH_SV39) {
        println!("_start {} over", _start as usize);
        return -1;
    }
    if _start % PAGE_SIZE != 0 {
        return -1;
    }

    if _len == 0 {
        return 0;
    }

    let end = match _start.checked_add(_len) {
        Some(end) => end,
        None => return -1,
    };
    let start_vpn = VirtAddr::from(_start).floor();
    let end_vpn = VirtAddr::from(end).ceil();

    let mut task = TASK_MANAGER.current_task();
    for vpn in start_vpn.0..end_vpn.0 {
        if task
            .memory_set
            .translate(crate::mm::VirtPageNum(vpn))
            .is_none()
        {
            return -1;
        }
    }
    if task
        .memory_set
        .remove_area(VirtAddr::from(_start), VirtAddr::from(end))
    {
        0
    } else {
        -1
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
