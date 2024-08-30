use std::fs::{self, File};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use ffmpeg_the_third as ffmpeg;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化FFmpeg
    ffmpeg::init()?;

    // 读取RTSP URL列表
    let urls = read_urls("rtsp.txt")?;

    // 创建video文件夹
    fs::create_dir_all("video")?;

    // 创建一个原子布尔值来控制程序运行
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // 创建一个线程来监听用户输入
    let input_thread = thread::spawn(move || {
        let mut input = String::new();
        while input.trim() != "q" {
            input.clear();
            if let Ok(_) = io::stdin().read_line(&mut input) {
                if input.trim() == "q" {
                    r.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }
    });

    // 为每个URL创建一个线程
    let handles: Vec<_> = urls
        .into_iter()
        .enumerate()
        .map(|(index, url)| {
            let running = running.clone();
            thread::spawn(move || process_stream(index, url, running))
        })
        .collect();

    // 等待输入线程完成（即用户按下'q'）
    input_thread.join().unwrap();

    println!("Stopping all streams...");

    // 等待所有工作线程完成
    for handle in handles {
        handle.join().unwrap();
    }

    println!("All streams stopped. Program exiting.");

    Ok(())
}

fn read_urls<P: AsRef<Path>>(path: P) -> io::Result<Vec<String>> {
    let file = File::open(path)?;
    let reader = io::BufReader::new(file);
    reader.lines().collect()
}

fn process_stream(id: usize, url: String, running: Arc<AtomicBool>) {
    println!("[Stream {}] Starting: {}", id, url);
    while running.load(Ordering::SeqCst) {
        match stream_to_file(id, &url, running.clone()) {
            Ok(_) => println!("[Stream {}] Ended for {}", id, url),
            Err(e) => eprintln!("[Stream {}] Error processing {}: {:?}", id, url, e),
        }
        if running.load(Ordering::SeqCst) {
            println!("[Stream {}] Retrying {} in 5 seconds...", id, url);
            thread::sleep(Duration::from_secs(5)); // 等待5秒后重试
        }
    }
    println!("[Stream {}] Stopped: {}", id, url);
}

fn stream_to_file(id: usize, url: &str, running: Arc<AtomicBool>) -> Result<(), String> {
    let mut ictx = ffmpeg::format::input(&url).map_err(|e| e.to_string())?;
    let input = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or_else(|| "No video stream found".to_string())?;
    let video_stream_index = input.index();

    let mut output_file =
        create_output_file(url).map_err(|e| format!("Failed to create output file: {}", e))?;
    let mut last_split = Instant::now();

    println!("[Stream {}] Started writing to file", id);

    for (stream, packet) in ictx.packets().filter_map(|r| r.ok()) {
        if !running.load(Ordering::SeqCst) {
            println!("[Stream {}] Stopping gracefully...", id);
            break;
        }

        if stream.index() == video_stream_index {
            if let Some(data) = packet.data() {
                output_file
                    .write_all(data)
                    .map_err(|e| format!("Failed to write packet data: {}", e))?;
            }

            if last_split.elapsed() >= Duration::from_secs(300) {
                // 5分钟
                output_file
                    .flush()
                    .map_err(|e| format!("Failed to flush file: {}", e))?;
                output_file = create_output_file(url)
                    .map_err(|e| format!("Failed to create new output file: {}", e))?;
                last_split = Instant::now();
                println!("[Stream {}] Created new file", id);
            }
        }
    }

    // 确保所有数据都写入磁盘
    output_file
        .flush()
        .map_err(|e| format!("Failed to flush final file: {}", e))?;
    println!("[Stream {}] Finished writing to file", id);

    Ok(())
}

fn create_output_file(url: &str) -> io::Result<File> {
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("video/{}_{}.mp4", url.replace("/", "_"), timestamp);
    File::create(filename)
}
