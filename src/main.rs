use std::{
    collections::VecDeque,
    fs,
    io::{Read, Write},
    mem,
    net::{TcpListener, TcpStream},
    rc::Rc,
    sync::{mpsc, Arc, Mutex},
    thread::{self, JoinHandle},
};

extern crate signal_hook;

fn main() {
    let listener = TcpListener::bind("127.0.0.1:7878").unwrap();
    let pool = ThreadPool::new(5);

    reg_for_sigs(Box::new(|| {
        pool.shutdown();
    }));

    for stream in listener.incoming() {
        let stream = stream.unwrap();

        pool.submit(|| {
            handle_connection(stream);
        });
    }
}

pub fn reg_for_sigs(func: Box<dyn Fn() -> () + Send + Sync>) {
    unsafe { signal_hook::register(signal_hook::SIGINT, || on_sigint(func)) }
        .and_then(|_| {
            println!("Registered for SIGINT");
            Ok(())
        })
        .or_else(|e| {
            println!("Failed to register for SIGINT {:?}", e);
            Err(e)
        })
        .ok();
}

fn on_sigint(func: Box<dyn Fn() -> ()>) {
    println!("SIGINT caught - exiting");
    func();
    std::process::exit(128 + signal_hook::SIGINT);
}

fn handle_connection(mut stream: TcpStream) {
    let get_req = b"GET / HTTP/1.1\r\n";
    let mut buffer = [0; 1024];
    let _ = stream.read(&mut buffer).unwrap();

    let (status_line, contents_file) = if buffer.starts_with(get_req) {
        ("HTTP/1.1 200 OK", "hello.html")
    } else {
        ("HTTP/1.1 404 NOT FOUND", "404.html")
    };

    let contents = fs::read_to_string(contents_file).unwrap();

    let response = format!(
        "{}\r\nContent-Length: {}\r\n\r\n{}",
        status_line,
        contents.len(),
        contents
    );

    let _ = stream.write(response.as_bytes()).unwrap();
    stream.flush().unwrap();

    // println!(
    //     "Request: {} {}",
    //     String::from_utf8_lossy(&buffer[..]),
    //     response.as_bytes().len()
    // );
}

type Job = Box<dyn FnOnce() + Send + 'static>;

enum Message {
    NewJob(Job),
    Terminate,
}

struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<Message>,
}

impl ThreadPool {
    /// Create a new ThreadPool
    ///
    /// nthreads is the number of threads in the pool
    /// # Panics
    ///
    pub fn new(nthreads: u16) -> ThreadPool {
        assert!(nthreads > 0);

        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(nthreads.into());
        for id in 0..nthreads {
            workers.push(Worker::new(id, Arc::clone(&receiver)));
        }

        ThreadPool {
            workers,
            sender: sender,
        }
    }

    pub fn submit<F>(&self, request: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.sender.send(Message::NewJob(Box::new(request)));
    }

    pub fn shutdown(&self) {
        println!("sending terminate messgae to all workers");
        for _ in 0..self.workers.len() {
            self.sender.send(Message::Terminate).unwrap();
        }

        println!("joining all workers on shutdown");
        self.workers.iter_mut().for_each(|worker| {
            if let Some(handle) = worker.joinHandle.take() {
                handle.join().unwrap();
            }
        });
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Worker {
    id: u16,
    joinHandle: Option<JoinHandle<()>>,
}

impl Worker {
    fn new(id: u16, receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
        let joinHandle = thread::spawn(move || loop {
            let message = receiver.lock().unwrap().recv().unwrap();

            match message {
                Message::NewJob(job) => {
                    println!("executing job on worker {}", id);
                    job();
                }

                Message::Terminate => {
                    println!("terminating worker {}", id);
                    break;
                }
            }
        });

        let joinHandle = Some(joinHandle);

        Worker { id, joinHandle }
    }
}
