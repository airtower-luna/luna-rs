use std::io::Error;
use nix::libc::timespec;

pub const ECHO_FLAG: u8 = 1;
pub const MIN_SIZE: usize = size_of::<u32>() + size_of::<timespec>() + size_of::<u8>();


/// Enable realtime scheduling for the current process. The offset is
/// the priority relative to the minimum realtime priority. Requires
/// CAP_SYS_NICE capability.
pub fn set_rt_prio(offset: i32) -> Result<(), Error> {
    let min_rt_prio = unsafe {
	libc::sched_get_priority_min(libc::SCHED_RR)
    };
    let max_rt_prio = unsafe {
	libc::sched_get_priority_max(libc::SCHED_RR)
    };
    let sparam = libc::sched_param{
	sched_priority: max_rt_prio.min(min_rt_prio + offset)
    };
    let ret = unsafe {
	libc::sched_setscheduler(0, libc::SCHED_RR, &sparam)
    };
    if ret < 0 {
	Err(Error::last_os_error())
    } else {
	Ok(())
    }
}
