use crate::lwip_binding::{
    err_enum_t_ERR_OK, err_t, pbuf, pbuf_free, tcp_arg, tcp_close, tcp_output, tcp_pcb, tcp_recv,
    tcp_write, TCP_WRITE_FLAG_COPY, err_enum_t_ERR_MEM, tcp_poll, err_enum_t_ERR_CONN, err_enum_t_ERR_USE, err_enum_t_ERR_ABRT, err_enum_t_ERR_RST, tcp_state_CLOSED, tcp_state_CLOSE_WAIT, tcp_state_CLOSING, tcp_recved, tcp_sent, tcp_state, tcp_state_FIN_WAIT_1, tcp_state_FIN_WAIT_2, tcp_abort, tcp_err,
};
use crate::tun::PtrWrapper;
use core::task::{Context, Poll};
use std::env::consts;
use std::process::abort;
use std::sync::Mutex;
use log::debug;
use rayon::ThreadPool;
use std::ffi::c_void;
use std::pin::Pin;
use std::{io::Result, task::Waker};
use tokio::io::{AsyncRead, AsyncWrite};

pub struct TcpConnection {
    pcb: *mut tcp_pcb,

    pcb_closed: bool,

    pool: std::sync::Arc<ThreadPool>,

    callback: Pin<Box<Mutex<Callback>>>,
}

unsafe impl Send for TcpConnection {}
unsafe impl Sync for TcpConnection {}

struct Callback {
    recv_waker: Option<Waker>,
    write_waker: Option<Waker>,
    unread: Vec<u8>,
    met_eof: bool,
    reset_by_peer: bool,
}

struct PBuf {
    pbuf: *mut pbuf,
}

impl PBuf {
    fn data(&self) -> &[u8] {
        unsafe {
            let ptr = (*self.pbuf).payload as *const c_void as *const u8;
            let len = usize::from((*self.pbuf).len);

            std::slice::from_raw_parts(ptr, len)
        }
    }
}

impl Drop for PBuf {
    fn drop(&mut self) {
        unsafe { pbuf_free(self.pbuf) };
    }
}

const SINGLE_CONNECTION_BUFFER_SIZE: usize = 1024 * 8 * 8;

extern "C" fn recv_function(
    arg: *mut std::os::raw::c_void,
    pcb: *mut tcp_pcb,
    p: *mut pbuf,
    err: err_t,
) -> err_t {
    // println!("Recv called time {:?}", std::time::Instant::now());
    if err != err_enum_t_ERR_OK as err_t {
        return err;
    }
    let callback = arg as *const Mutex<Callback>;
    let callback = unsafe { &*callback };

    if p.is_null() {
        callback.lock().unwrap().met_eof = true;
        unsafe { tcp_recved(pcb, 0) };
        return err_enum_t_ERR_OK as err_t;
    }


    let pbuf = PBuf { pbuf: p };

    let pbuf_data = pbuf.data();

    {
        let locked = &mut callback.lock().unwrap().unread;

        if locked.capacity() - locked.len() < pbuf_data.len() {
            std::mem::forget(pbuf);
            return err_enum_t_ERR_MEM as err_t;
        }

        locked.extend_from_slice(pbuf_data);
    }

    let recv_waker = &mut callback.lock().unwrap().recv_waker;

    unsafe { tcp_recved(pcb, pbuf_data.len() as u16) };

    if let Some(waker) = recv_waker.take() {
        waker.wake();
    } else {
        // println!("Calling Recv without waker");
    }

    return err_enum_t_ERR_OK as err_t;
}

extern "C" fn poll_function(
    arg: *mut std::os::raw::c_void,
    _: *mut tcp_pcb,
) -> err_t {
    let callback = arg as *const Mutex<Callback>;
    let callback = unsafe { &*callback };

    let write_waker = &mut callback.lock().unwrap().write_waker;

    if let Some(waker) = write_waker.take() {
        waker.wake();
    } else {
        // println!("Polling without waker");
    }

    return err_enum_t_ERR_OK as err_t;
}

extern "C" fn err_function(
    arg: *mut std::os::raw::c_void,
    _: err_t
) {
    let callback = arg as *const Mutex<Callback>;
    let callback = unsafe { &*callback };

    callback.lock().unwrap().reset_by_peer = true;
}

extern "C" fn sent_function(
    arg: *mut std::os::raw::c_void,
    _: *mut tcp_pcb,
    _: u16
) -> err_t {
    // println!("Sent called");
    let callback = arg as *const Mutex<Callback>;
    let callback = unsafe { &*callback };

    let write_waker = &mut callback.lock().unwrap().write_waker;

    if let Some(waker) = write_waker.take() {
        waker.wake();
    } else {
        // println!("Calling Sent without waker");
    }

    return err_enum_t_ERR_OK as err_t;
}

impl TcpConnection {
    pub fn new(pcb: *mut tcp_pcb, pool: std::sync::Arc<ThreadPool>) -> TcpConnection {
        unsafe { assert!((*pcb).state != tcp_state_CLOSED) };
        let callback = Callback {
            recv_waker: None,
            write_waker: None,
            unread: Vec::with_capacity(SINGLE_CONNECTION_BUFFER_SIZE),
            met_eof: false,
            reset_by_peer: false,
        };
        let mut pinned = Box::pin(Mutex::new(callback));
        let ptr = unsafe { pinned.as_mut().get_unchecked_mut() as *mut Mutex<Callback> };

        let recv_callback_wrapper = PtrWrapper(ptr);
        let pcb_wrapper = PtrWrapper(pcb);

        pool.install(|| {
            let pcb_wrapper = pcb_wrapper;
            let recv_callback_wrapper = recv_callback_wrapper;

            let pcb = pcb_wrapper.0;
            let ptr = recv_callback_wrapper.0;

            unsafe {
                tcp_arg(pcb, ptr as *mut c_void);

                tcp_poll(pcb, Some(poll_function), 1);
                tcp_sent(pcb, Some(sent_function));

                tcp_recv(pcb, Some(recv_function));
                tcp_err(pcb, Some(err_function));
            }
        });

        TcpConnection {
            pcb,
            pool,
            pcb_closed: false,
            callback: pinned,
        }
    }
}

impl AsyncRead for TcpConnection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        {
            let waker = cx.waker().clone();
            let callback = &self.as_mut().callback;
            callback.lock().unwrap().recv_waker.replace(waker);
        }

        let mut locked_callback = self.callback.lock().unwrap();
        let locked_unread = &mut locked_callback.unread;

        if locked_unread.is_empty() {
            if locked_callback.met_eof {
                return Poll::Ready(Ok(()));
            }

            return Poll::Pending;
        } else {

            let mut need_call_waker_again = false;

            let read_size;
            if buf.remaining() < locked_unread.len() {
                read_size = buf.remaining();
                need_call_waker_again = true;
            } else {
                read_size = locked_unread.len();
            }

            {
                let sent_data = locked_unread.drain(..read_size);

                buf.put_slice(sent_data.as_slice());

                // println!("Read data from tcp tun {} {:?}", read_size, std::time::Instant::now());
            }

            if locked_callback.met_eof {
                need_call_waker_again = true;
            }

            if need_call_waker_again {
                cx.waker().wake_by_ref();
            }
            return Poll::Ready(Ok(()));
        }
    }
}

impl AsyncWrite for TcpConnection {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        // debug!("Poll write len {}", buf.len());

        let pcb_wrapper = PtrWrapper(self.pcb);
        {
            let waker = cx.waker().clone();
            let callback = &self.as_mut().callback;
            callback.lock().unwrap().write_waker.replace(waker);
        }

        let pool = &self.pool;

        let result = pool.install(|| unsafe {
            let pcb_wrapper = pcb_wrapper;
            let err = match_tcp_state_to_io_error_kind((*pcb_wrapper.0).state);
            if let Some(err) = err {
                // if err == std::io::ErrorKind::ConnectionAborted {
                //     return Poll::Ready(Ok(0));
                // }
                return Poll::Ready(Err(std::io::Error::new(err, "tcp state is not connected")));
            }

            let err_t = tcp_write(
                pcb_wrapper.0,
                buf.as_ptr() as *const c_void,
                buf.len() as u16,
                TCP_WRITE_FLAG_COPY as u8,
            );
            // println!("tcp write result {}", err_t);

            if err_t == err_enum_t_ERR_MEM as err_t {
                // data is not writen.
                tcp_output(pcb_wrapper.0);
                Poll::Pending
            } else if err_t == err_enum_t_ERR_OK as err_t{
                Poll::Ready(Ok(buf.len()))
            } else {
                let err_kind = match_error_to_rust_error_kind(err_t);
                Poll::Ready(Err(std::io::Error::new(
                    err_kind.unwrap(),
                    format!("tcp_write failed {}", err_t),
                )))
            }

        });

        result
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<()>> {
        let pool = &self.pool;
        let pcb_wrapper = PtrWrapper(self.pcb);

        let err_t = pool.install(|| {
            let pcb_wrapper = pcb_wrapper;

            unsafe { tcp_output(pcb_wrapper.0) }
        });

        if err_t == err_enum_t_ERR_OK as err_t {
            Poll::Ready(Ok(()))
        } else {
            let err_kind = match_error_to_rust_error_kind(err_t);
            Poll::Ready(Err(std::io::Error::new(
                err_kind.unwrap(),
                format!("flush failed {}", err_t),
            )))
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<()>> {
        let pcb_wrapper = PtrWrapper(self.pcb);
        debug!("PCB shutdown");

        unsafe {
            if (*pcb_wrapper.0).state == tcp_state_CLOSED {
                return Poll::Ready(Ok(()));
            }
        }

        let pool = &self.pool;

        let reset_by_peer = {
            self.callback.lock().unwrap().reset_by_peer
        };

        if reset_by_peer {
            return Poll::Ready(Ok(()));
        }

        let err_t = pool.install(|| unsafe {
            let pcb_wrapper = pcb_wrapper;

            close_tcp_in_shutdown(pcb_wrapper.0)
        });

        self.as_mut().pcb_closed = true;
        if err_t == err_enum_t_ERR_OK as err_t {
            Poll::Ready(Ok(()))
        } else {
            let err_kind = match_error_to_rust_error_kind(err_t);
            Poll::Ready(Err(std::io::Error::new(
                err_kind.unwrap(),
                format!("poll shutdown failed {}", err_t),
            )))
        }
    }
}

unsafe fn close_tcp_in_shutdown(pcb: *mut tcp_pcb) -> i8 {
    tcp_close(pcb)
}

impl Drop for TcpConnection {
    fn drop(&mut self) {
        let reset_by_peer = {
            self.callback.lock().unwrap().reset_by_peer
        };

        let closed = self.pcb_closed || reset_by_peer;
        unsafe {
            let pcb_wrapper = PtrWrapper(self.pcb);

            self.pool.install(|| {
                let pcb_wrapper = pcb_wrapper;

                if !closed {
                    tcp_abort(pcb_wrapper.0);
                }
            });
        }
    }
}

fn match_tcp_state_to_io_error_kind(state: tcp_state) -> Option<std::io::ErrorKind> {
    match state {
        tcp_state_CLOSED => Some(std::io::ErrorKind::ConnectionAborted),
        tcp_state_CLOSE_WAIT => Some(std::io::ErrorKind::ConnectionAborted),
        tcp_state_CLOSING => Some(std::io::ErrorKind::ConnectionAborted),
        tcp_state_FIN_WAIT_1 => Some(std::io::ErrorKind::ConnectionAborted),
        tcp_state_FIN_WAIT_2 => Some(std::io::ErrorKind::ConnectionAborted),
        _ => None
    }
}

fn match_error_to_rust_error_kind(err: err_t) -> Option<std::io::ErrorKind> {
    let err = err as i32;
    match err {
        err_enum_t_ERR_OK => None,
        err_enum_t_ERR_MEM => Some(std::io::ErrorKind::OutOfMemory),
        err_enum_t_ERR_USE => Some(std::io::ErrorKind::AddrInUse),
        err_enum_t_ERR_ABRT => Some(std::io::ErrorKind::ConnectionAborted),
        err_enum_t_ERR_RST => Some(std::io::ErrorKind::ConnectionReset),
        err_enum_t_Err_CLSD => Some(std::io::ErrorKind::ConnectionAborted),
        err_enum_t_ERR_CONN => Some(std::io::ErrorKind::NotConnected),
        _ => {
            Some(std::io::ErrorKind::Other)
        }

    }

}
