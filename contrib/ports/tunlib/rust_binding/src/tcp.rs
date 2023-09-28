use crate::lwip_binding::{
    err_enum_t_ERR_OK, err_t, pbuf, pbuf_free, tcp_arg, tcp_close, tcp_output, tcp_pcb, tcp_recv,
    tcp_write, TCP_WRITE_FLAG_COPY, err_enum_t_ERR_MEM, tcp_poll, err_enum_t_ERR_CONN, err_enum_t_ERR_BUF, err_enum_t_ERR_USE, err_enum_t_ERR_ALREADY, err_enum_t_ERR_ABRT, err_enum_t_ERR_CLSD, err_enum_t_ERR_RST, err_enum_t_ERR_ARG, tcp_state_CLOSED, tcp_state_CLOSE_WAIT, tcp_state_CLOSING, tcp_recved, tcp_sent, tcp_abort,
};
use crate::tun::PtrWrapper;
use core::task::{Context, Poll};
use std::sync::Mutex;
use rayon::ThreadPool;
use std::ffi::c_void;
use std::pin::Pin;
use std::{io::Result, task::Waker};
use tokio::io::{AsyncRead, AsyncWrite};

pub struct TcpConnection {
    pcb: *mut tcp_pcb,

    pcb_closed: bool,

    pool: std::sync::Arc<ThreadPool>,

    callback: Pin<Box<Callback>>,
}

unsafe impl Send for TcpConnection {}
unsafe impl Sync for TcpConnection {}

struct Callback {
    recv_waker: Mutex<Option<Waker>>,
    write_waker: Mutex<Option<Waker>>,
    unread: Mutex<Vec<u8>>,
    met_eof: bool,
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
    let callback = arg as *mut Callback;
    let callback = unsafe { &mut *callback };

    if p.is_null() {
        callback.met_eof = true;
        unsafe { tcp_recved(pcb, 0) };
        return err_enum_t_ERR_OK as err_t;
    }

    let pbuf = PBuf { pbuf: p };

    let pbuf_data = pbuf.data();

    {
        let locked = callback.unread.lock();
        if let Ok(mut locked) = locked {
            locked.extend_from_slice(pbuf_data);
        } else {
            assert!(false, "{}", locked.err().unwrap());
        }
    }

    let mut recv_waker = callback.recv_waker.lock().unwrap();

    if let Some(waker) = recv_waker.take() {
        waker.wake();
    } else {
        // println!("Not waking");
    }

    unsafe { tcp_recved(pcb, pbuf_data.len() as u16) };
    return err_enum_t_ERR_OK as err_t;
}

extern "C" fn poll_function(
    arg: *mut std::os::raw::c_void,
    _: *mut tcp_pcb,
) -> err_t {
    let callback = arg as *mut Callback;
    let callback = unsafe { &mut *callback };

    let mut write_waker = callback.write_waker.lock().unwrap();

    if let Some(waker) = write_waker.take() {
        waker.wake();
    } else {
        // println!("Polling without waker");
    }

    return err_enum_t_ERR_OK as err_t;
}

extern "C" fn sent_function(
    arg: *mut std::os::raw::c_void,
    _: *mut tcp_pcb,
    _: u16
) -> err_t {
    // println!("Sent called");
    let callback = arg as *mut Callback;
    let callback = unsafe { &mut *callback };

    let mut write_waker = callback.write_waker.lock().unwrap();

    if let Some(waker) = write_waker.take() {
        waker.wake();
    } else {
        // println!("Calling Sent without waker");
    }

    return err_enum_t_ERR_OK as err_t;
}

impl TcpConnection {
    pub fn new(pcb: *mut tcp_pcb, pool: std::sync::Arc<ThreadPool>) -> TcpConnection {
        let callback = Callback {
            recv_waker: Mutex::new(None),
            write_waker: Mutex::new(None),
            unread: Mutex::new(Vec::new()),
            met_eof: false,
        };
        let mut pinned = Box::pin(callback);
        let ptr = unsafe { pinned.as_mut().get_unchecked_mut() as *mut Callback };

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

                tcp_recv(pcb, Some(recv_function))
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
        // println!("Poll read");

        {
            let waker = cx.waker().clone();
            let callback = &self.as_mut().callback;
            callback.recv_waker.lock().unwrap().replace(waker);
        }

        let mut locked = self.callback.unread.lock().unwrap();

        if locked.is_empty() {
            if self.callback.met_eof {
                return Poll::Ready(Ok(()));
            }

            return Poll::Pending;
        } else {

            let mut need_call_waker_again = false;

            let read_size;
            if buf.remaining() < locked.len() {
                read_size = buf.remaining();
                need_call_waker_again = true;
            } else {
                read_size = locked.len();
            }

            {
                let sent_data = locked.drain(..read_size);

                buf.put_slice(sent_data.as_slice());

                // println!("Read data from tcp tun {} {:?}", read_size, std::time::Instant::now());
            }

            if self.callback.met_eof {
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
        // println!("Poll write len {}", buf.len());
        let pcb_wrapper = PtrWrapper(self.pcb);
        {
            let waker = cx.waker().clone();
            let callback = &self.as_mut().callback;
            callback.write_waker.lock().unwrap().replace(waker);
        }

        let pool = &self.pool;

        let result = pool.install(|| unsafe {
            let pcb_wrapper = pcb_wrapper;

            let err_t = tcp_write(
                pcb_wrapper.0,
                buf.as_ptr() as *const c_void,
                buf.len() as u16,
                TCP_WRITE_FLAG_COPY as u8,
            );
            println!("tcp write result {}", err_t);

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

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<()>> {
        let pcb_wrapper = PtrWrapper(self.pcb);

        let pool = &self.pool;

        let err_t = pool.install(|| unsafe {
            let pcb_wrapper = pcb_wrapper;

            tcp_close(pcb_wrapper.0)
        });

        if err_t == err_enum_t_ERR_OK as err_t {
            self.get_mut().pcb_closed = true;
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

impl Drop for TcpConnection {
    fn drop(&mut self) {
        unsafe {
            println!("tcp drop");

            let pcb_wrapper = PtrWrapper(self.pcb);

            self.pool.install(|| {
                let pcb_wrapper = pcb_wrapper;

                let state = (*pcb_wrapper.0).state;

                match state {
                    tcp_state_CLOSED | tcp_state_CLOSE_WAIT | tcp_state_CLOSING => {
                        return;
                    }
                    _ => {
                        tcp_close(pcb_wrapper.0);
                        return;
                    }
                }
            });
        }
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
