use crate::lwip_binding::{
    err_enum_t_ERR_OK, err_t, pbuf, pbuf_free, tcp_arg, tcp_close, tcp_output, tcp_pcb, tcp_recv,
    tcp_write, TCP_WRITE_FLAG_COPY, err_enum_t_ERR_MEM, tcp_poll, err_enum_t_ERR_CONN, err_enum_t_ERR_BUF, err_enum_t_ERR_USE, err_enum_t_ERR_ALREADY, err_enum_t_ERR_ABRT, err_enum_t_ERR_CLSD, err_enum_t_ERR_RST, err_enum_t_ERR_ARG,
};
use crate::tun::PtrWrapper;
use core::task::{Context, Poll};
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
    recv_waker: Option<Waker>,
    write_waker: Option<Waker>,
    unread: Vec<u8>,
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
    _: *mut tcp_pcb,
    p: *mut pbuf,
    err: err_t,
) -> err_t {
    println!("Recv called");
    if err != err_enum_t_ERR_OK as err_t {
        return err;
    }
    let callback = arg as *mut Callback;
    let callback = unsafe { &mut *callback };

    if p.is_null() {
        callback.met_eof = true;
        return err_enum_t_ERR_OK as err_t;
    }

    let pbuf = PBuf { pbuf: p };

    callback.unread.extend_from_slice(pbuf.data());

    if let Some(waker) = callback.recv_waker.take() {
        waker.wake();
    } else {
        println!("Not waking");
    }

    return err_enum_t_ERR_OK as err_t;
}

extern "C" fn poll_function(
    arg: *mut std::os::raw::c_void,
    _: *mut tcp_pcb,
) -> err_t {
    println!("Poll called");
    let callback = arg as *mut Callback;
    let callback = unsafe { &mut *callback };

    if let Some(waker) = callback.write_waker.take() {
        waker.wake();
    } else {
        println!("Polling without waker");
    }

    return err_enum_t_ERR_OK as err_t;
}

impl TcpConnection {
    pub fn new(pcb: *mut tcp_pcb, pool: std::sync::Arc<ThreadPool>) -> TcpConnection {
        let callback = Callback {
            recv_waker: None,
            write_waker: None,
            unread: Vec::new(),
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
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        println!("Poll read");
        if self.callback.unread.is_empty() {
            if self.callback.met_eof {
                return Poll::Ready(Ok(()));
            }

            self.get_mut().callback.recv_waker = Some(cx.waker().clone());
            return Poll::Pending;
        } else {
            let mut_self = self.get_mut();

            let read_size;
            if buf.remaining() < mut_self.callback.unread.len() {
                read_size = buf.remaining();
            } else {
                read_size = mut_self.callback.unread.len();
            }

            {
                let sent_data = mut_self.callback.unread.drain(..read_size);

                buf.put_slice(sent_data.as_slice());

                println!("Read data from tcp tun {:?}", std::str::from_utf8(buf.filled()));
            }

            if mut_self.callback.met_eof {
                cx.waker().wake_by_ref();
            }
            return Poll::Ready(Ok(()));
        }
    }
}

impl AsyncWrite for TcpConnection {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        println!("Poll write len {}", buf.len());
        let pcb_wrapper = PtrWrapper(self.pcb);
        self.as_mut().callback.write_waker = Some(cx.waker().clone());

        let pool = &self.pool;

        let result = pool.install(|| unsafe {
            let pcb_wrapper = pcb_wrapper;

            let mut err_t = tcp_write(
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

// impl Drop for TcpConnection {
//     fn drop(&mut self) {
//         unsafe {
//             if !self.pcb_closed {
//                 println!("tcp drop");

//                 let pcb_wrapper = PtrWrapper(self.pcb);

//                 self.pool.install(|| {
//                     let pcb_wrapper = pcb_wrapper;

//                     tcp_close(pcb_wrapper.0)
//                 });
//             }
//         }
//     }
// }

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
