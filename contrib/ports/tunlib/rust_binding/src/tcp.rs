use crate::lwip_binding::{
    err_enum_t_ERR_OK, err_t, pbuf, pbuf_free, tcp_arg, tcp_close, tcp_output, tcp_pcb, tcp_recv,
    tcp_write, TCP_WRITE_FLAG_COPY,
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

    recv_callback: Pin<Box<RecvCallback>>,
}

unsafe impl Send for TcpConnection {}
unsafe impl Sync for TcpConnection {}

struct RecvCallback {
    waker: Option<Waker>,
    unread: Vec<Vec<u8>>,
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
    if err != err_enum_t_ERR_OK as err_t {
        return err;
    }
    let callback = arg as *mut RecvCallback;
    let callback = unsafe { &mut *callback };

    if p.is_null() {
        callback.met_eof = true;
        return err_enum_t_ERR_OK as err_t;
    }

    let pbuf = PBuf { pbuf: p };

    callback.unread.push(pbuf.data().to_vec());

    if let Some(waker) = callback.waker.take() {
        waker.wake();
    }

    return err_enum_t_ERR_OK as err_t;
}

impl TcpConnection {
    pub fn new(pcb: *mut tcp_pcb, pool: std::sync::Arc<ThreadPool>) -> TcpConnection {
        let callback = RecvCallback {
            waker: None,
            unread: Vec::new(),
            met_eof: false,
        };
        let mut pinned = Box::pin(callback);
        let ptr = unsafe { pinned.as_mut().get_unchecked_mut() as *mut RecvCallback };

        let recv_callback_wrapper = PtrWrapper(ptr);
        let pcb_wrapper = PtrWrapper(pcb);

        pool.install(|| {
            let pcb_wrapper = pcb_wrapper;
            let recv_callback_wrapper = recv_callback_wrapper;

            let pcb = pcb_wrapper.0;
            let ptr = recv_callback_wrapper.0;

            unsafe {
                tcp_arg(pcb, ptr as *mut c_void);

                tcp_recv(pcb, Some(recv_function))
            }
        });

        TcpConnection {
            pcb,
            pool,
            pcb_closed: false,
            recv_callback: pinned,
        }
    }
}

impl AsyncRead for TcpConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.recv_callback.unread.is_empty() {
            if self.recv_callback.met_eof {
                return Poll::Ready(Ok(()));
            }

            self.get_mut().recv_callback.waker = Some(cx.waker().clone());
            return Poll::Pending;
        } else {
            let mut_self = self.get_mut();

            let range = 0..mut_self.recv_callback.unread.len();

            for data in mut_self.recv_callback.unread.drain(range) {
                println!("Read data {}", std::str::from_utf8(&data).unwrap());
                buf.put_slice(&data);
            }

            if mut_self.recv_callback.met_eof {
                cx.waker().wake_by_ref();
            }
            return Poll::Ready(Ok(()));
        }
    }
}

impl AsyncWrite for TcpConnection {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        println!("Writing data {}", std::str::from_utf8(buf).unwrap());

        let pcb_wrapper = PtrWrapper(self.pcb);

        let pool = &self.pool;

        let err_t = pool.install(|| unsafe {
            let pcb_wrapper = pcb_wrapper;

            tcp_write(
                pcb_wrapper.0,
                buf.as_ptr() as *const c_void,
                buf.len() as u16,
                TCP_WRITE_FLAG_COPY as u8,
            )
        });

        if err_t == err_enum_t_ERR_OK as err_t {
            Poll::Ready(Ok(buf.len()))
        } else {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("tcp_write failed {}", err_t),
            )))
        }
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
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("tcp_output failed {}", err_t),
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
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("tcp_close failed {}", err_t),
            )))
        }
    }
}

impl Drop for TcpConnection {
    fn drop(&mut self) {
        unsafe {
            if !self.pcb_closed {
                println!("tcp drop");

                let pcb_wrapper = PtrWrapper(self.pcb);

                self.pool.install(|| {
                    let pcb_wrapper = pcb_wrapper;

                    tcp_close(pcb_wrapper.0)
                });
            }
        }
    }
}
