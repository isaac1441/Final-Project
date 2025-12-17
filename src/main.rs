use std::thread;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use crossbeam::channel::{unbounded, Sender, Receiver};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;


type Job = Box<dyn FnOnce() +Send + 'static>;

//error handling
#[derive(Debug, Clone)]
pub struct ProcessingError {
    file_path: String,
    error_type: String,
    description: String,
}

//error handling implementation
impl ProcessingError {
    fn new(file_path: &str, error_type: &str, description: &str) -> Self {
        ProcessingError {
            file_path: file_path.to_string(),
            error_type: error_type.to_string(),
            description: description.to_string(),
        }
    }
}

// file stats & anaylsis structs
pub struct FileStats {
    word_count: usize,
    line_count: usize,
    char_frequencies: HashMap<char, usize>,
    size_bytes: u64,
}

pub struct FileAnalysis {
    filename: String,
    stats: FileStats,
    errors: Vec<ProcessingError>,
    processing_time: Duration,
}

// generic thread pool with cancel flag
pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: Sender<Job>,
    stop_flag: Arc<AtomicBool>,
}

impl ThreadPool {
    // create a new thread pool with dynamic # of workers
    fn new(size: usize) -> ThreadPool {
        let (sender, receiver) = unbounded();
        let receiver = Arc::new(Mutex::new(receiver));

        let stop_flag = Arc::new(AtomicBool::new(false));
        
        let workers = (0..size)
            .map(|id| {
                Worker::new(
                    id,
                    Arc::clone(&receiver),
                    Arc::clone(&stop_flag),
                )
            })
            .collect();

        ThreadPool { workers, sender, stop_flag }
    }

    // execute a task 
    fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if !self.stop_flag.load(Ordering::Relaxed) {
            let job = Box::new(f);
            let _ = self.sender.send(job);
        }
    }

    // proper shutdown mechanism
    fn wait_for_completion(self) {
        drop(self.sender);
        
        self.stop_flag.store(true, Ordering::Relaxed);
        
        for worker in self.workers {
            worker.join().unwrap_or_else(|_| {
                eprintln!("Worker panicked during shutdown");
            });
        }
    }
}

pub struct Worker {
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(
        id: usize,
        receiver: Arc<Mutex<Receiver<Job>>>,
        stop_flag: Arc<AtomicBool>,
    ) -> Worker {
        let thread = thread::spawn(move || {
            loop {
                let job = {
                    let receiver_guard = receiver.lock().unwrap();
                    match receiver_guard.recv_timeout(Duration::from_millis(100)) {
                        Ok(job) => job,
                        Err(_) => {
                            if stop_flag.load(Ordering::Relaxed) {
                                break;
                            }
                            continue;
                        }
                    }
                };
                job();
            }
            println!("Worker {} shutting down",id);
        });

        Worker { thread: Some(thread) }
    }
    fn join(mut self) -> thread::Result<()> {
        self.thread.take().unwrap().join()
    }
}
//file processing features

// processing files from multiple directories
pub fn file_collector(directories: Vec<&str>) -> Vec<PathBuf> {
    let mut all_files = Vec::new();
    
    for dir in directories {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() && path.extension().map(|e| e == "txt").unwrap_or(false) {
                        all_files.push(path);
                    }
                }
            }
        }
    }
    
    all_files
}

//analyizer for Word count, Line count, Character frequency, File size statistics 
pub fn process_file(file_path: &Path) -> FileAnalysis {
    // processing time
    let start_time = Instant::now();
    let mut errors = Vec::new();
    
    let file_size = match fs::metadata(file_path) {
        Ok(metadata) => metadata.len(),
        Err(e) => {
            errors.push(ProcessingError::new(
                &file_path.to_string_lossy(),
                "MetadataError",
                &format!("Failed to get file metadata: {}", e)
            ));
            0
        }};

    let file_content = match fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(e) => {
            errors.push(ProcessingError::new(
                &file_path.to_string_lossy(),
                "ReadError",
                &format!("Failed to read file: {}", e)
            ));
            return FileAnalysis {
                filename: file_path.to_string_lossy().to_string(),
                stats: FileStats {
                    word_count: 0,
                    line_count: 0,
                    char_frequencies: HashMap::new(),
                    size_bytes: file_size,
                },
                errors,
                processing_time: start_time.elapsed(),
            };
        }
    };

    let mut word_count = 0;
    let mut line_count = 0;
    let mut char_frequencies = HashMap::new();

    for line in file_content.lines() {
        line_count += 1;
        word_count += line.split_whitespace().count();
        for c in line.chars() {
            *char_frequencies.entry(c).or_insert(0) += 1;
        }
    }

    let processing_time = start_time.elapsed();

    FileAnalysis {
        filename: file_path.to_string_lossy().to_string(),
        stats: FileStats {
            word_count,
            line_count,
            char_frequencies,
            size_bytes: file_size,
        },
        errors,
        processing_time,
    }
}

// progress tracking 

pub struct Progress {
    processed_files: Arc<Mutex<usize>>,
    total_files: usize,
    stop_flag: Arc<AtomicBool>,
}

// real time progress updates
pub fn track_progress(progress: Arc<Progress>){
    while !progress.stop_flag.load(Ordering::Relaxed){
        let processed_files = *progress.processed_files.lock().unwrap();
        let percentage = (processed_files as f64 / progress.total_files as f64) * 100.0;
        println!("Progress: {:.2}% ({} of {} files)", percentage, processed_files, progress.total_files);

        if processed_files >= progress.total_files{
            break;
        }

        thread::sleep(Duration::from_secs(1));
    }
    println!("Progress tracking complete");
}

// per file processing status
pub fn process_and_track(progress: Arc<Progress>, file_path: PathBuf){
    let result = process_file(&file_path);
    
    
    if result.errors.is_empty() {
        println!(
            "Processed {}: 
            {} words, {} lines, {} bytes in {:?}",
            result.filename,
            result.stats.word_count,
            result.stats.line_count,
            result.stats.size_bytes,
            result.processing_time
        );

        if !result.stats.char_frequencies.is_empty() {
            let mut sorted: Vec<_> = result.stats.char_frequencies.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1));
            println!("  Most common characters:");
            for (ch, count) in sorted.iter().take(3) {
                println!("    '{}': {}", ch, count);
            }
        }

    } else {
        //error reporting with context
        for error in &result.errors {
            println!("Error processing {}: [{}] {}", error.file_path, error.error_type, error.description);
        }
    }
    
    let mut processed_files = progress.processed_files.lock().unwrap();
    *processed_files += 1;
}

fn main() {
    println!("Starting parallel file processor...");

    let directories = vec![".", "books", "data"];
    let file_paths = file_collector(directories);
    
    if file_paths.is_empty() {
        println!("No .txt files found in specified directories");
        return;
    }

    let total_files = file_paths.len();
    println!("Found {} files to process", total_files);

    let processed_files = Arc::new(Mutex::new(0));
    let stop_flag = Arc::new(AtomicBool::new(false));

    let progress = Arc::new(Progress {
        processed_files: processed_files.clone(),
        total_files,
        stop_flag: stop_flag.clone(),
    });

    let num_workers = 4;
    let pool = ThreadPool::new(num_workers);
    println!("Created thread pool with {} workers", num_workers);
    
    let progress_for_tracker = progress.clone();
    let progress_tracker = thread::spawn(move || {
        track_progress(progress_for_tracker);
    });
    
    for file_path in file_paths {
        let progress_clone = progress.clone();
        pool.execute(move || {
            process_and_track(progress_clone, file_path);
        });
    }
    
    println!("Waiting for file processing to complete...");
    pool.wait_for_completion();
    
    stop_flag.store(true, Ordering::Relaxed);
    progress_tracker.join().unwrap_or_else(|_| {
        eprintln!("Progress tracker thread panicked");
    });
    
    println!("\nFile processing complete!");
    let final_count = *processed_files.lock().unwrap();
    println!("Processed {} of {} files", final_count, total_files);
    
    if final_count != total_files {
        eprintln!("WARNING: Only processed {} out of {} files!", final_count, total_files);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::io::Write;
    use tempfile::NamedTempFile;
    
    //integration tests
    #[test]
    fn test_thread_pool_creation() {
        let pool = ThreadPool::new(2);
    }
    
    #[test]
    fn test_thread_pool_executes_tasks() {
        let pool = ThreadPool::new(2);
        let counter = Arc::new(AtomicUsize::new(0));
        
        for _ in 0..5 {
            let counter = Arc::clone(&counter);
            pool.execute(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            });
        }
        
        pool.wait_for_completion();
        assert_eq!(counter.load(Ordering::SeqCst), 5);
    }
    
    #[test]
    fn test_thread_pool_shutdown() {
        let pool = ThreadPool::new(2);
        let counter = Arc::new(AtomicUsize::new(0));
        
        for _ in 0..3 {
            let counter = Arc::clone(&counter);
            pool.execute(move || {
                std::thread::sleep(std::time::Duration::from_millis(10));
                counter.fetch_add(1, Ordering::SeqCst);
            });
        }
        
        pool.wait_for_completion();
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }
    
    //integration tests
    #[test]
    fn test_file_processing_basic() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "Hello World\nThis is a test").unwrap();
        
        let result = process_file(file.path());
        assert_eq!(result.stats.word_count, 6);
        assert_eq!(result.stats.line_count, 2);
        assert!(result.errors.is_empty());
        assert!(result.processing_time > Duration::from_secs(0));
    }
    
    #[test]
    fn test_file_processing_empty() {
        let file = NamedTempFile::new().unwrap();
        let result = process_file(file.path());
        assert_eq!(result.stats.word_count, 0);
        assert_eq!(result.stats.line_count, 0);
        assert!(result.errors.is_empty());
    }
    
    #[test]
    fn test_file_processing_with_errors() {
        let result = process_file(std::path::Path::new("non_existent_file.txt"));
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].error_type.contains("Error"));
        assert_eq!(result.stats.word_count, 0);
        assert_eq!(result.stats.line_count, 0);
    }
    // error handling 
    #[test]
    fn test_character_frequency() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "aaa bbb ccc").unwrap();
        
        let result = process_file(file.path());
        assert_eq!(result.stats.char_frequencies.get(&'a'), Some(&3));
        assert_eq!(result.stats.char_frequencies.get(&'b'), Some(&3));
        assert_eq!(result.stats.char_frequencies.get(&'c'), Some(&3));
        assert_eq!(result.stats.char_frequencies.get(&' '), Some(&2));
    }
    
    #[test]
    fn test_error_handling_multiple_files() {
        let file1 = NamedTempFile::new().unwrap();
        let file2 = NamedTempFile::new().unwrap();
        
        let result1 = process_file(file1.path());
        let result2 = process_file(file2.path());
        let result3 = process_file(Path::new("nonexistent.txt"));
        
        assert!(result1.errors.is_empty());
        assert!(result2.errors.is_empty());
        assert!(!result3.errors.is_empty());
    }
    
    #[test]
    fn test_concurrent_file_processing() {
        let pool = ThreadPool::new(2);
        let results = Arc::new(Mutex::new(Vec::new()));
        
        for i in 0..3{
            let results = Arc::clone(&results);
            pool.execute(move || {
                let mut file = NamedTempFile::new().unwrap();
                write!(file, "Test file {}", i).unwrap();
                let result = process_file(file.path());
                results.lock().unwrap().push(result);
            });
        }
        pool.wait_for_completion();
        let final_results = results.lock().unwrap();
        assert_eq!(final_results.len(), 3);
        for result in final_results.iter() {
            assert!(result.errors.is_empty());
            assert!(result.stats.word_count > 0);
        }
    }
}