#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::process::Command;
use sysinfo::System;
use base64::Engine;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[derive(Serialize, Deserialize, Debug)]
struct HardwareSpec {
    cpu: String,
    cpu_cores: u32,
    cpu_threads: u32,
    cpu_avx2: bool,
    cpu_avx512: bool,
    ram_gb: u32,
    gpu: Option<String>,
    vram_gb: Option<f32>,
    os: String,
    chip: Option<String>,
    bandwidth_gbps: Option<f32>,
    storage_total_gb: Option<f32>,
    storage_free_gb: Option<f32>,
    storage_type: Option<String>,
}

#[cfg(target_os = "windows")]
fn detect_gpu_dxgi() -> Option<(String, f32, f32)> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_DESC1};
    use windows::core::Interface;
    
    unsafe {
        let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
            Ok(f) => f,
            Err(_) => return None,
        };
        
        let mut adapter_index = 0;
        loop {
            let adapter = match factory.EnumAdapters1(adapter_index) {
                Ok(a) => a,
                Err(_) => break,
            };
            
            let mut desc = DXGI_ADAPTER_DESC1::default();
            if adapter.GetDesc1(&mut desc as *mut _).is_ok() {
                // Skip software adapters
                if desc.Flags & 0x2 != 0 {
                    adapter_index += 1;
                    continue;
                }
                
                let vram_bytes = desc.DedicatedVideoMemory as f32;
                let vram_gb = vram_bytes / (1024.0 * 1024.0 * 1024.0);
                
                // Only return if VRAM is reasonable (> 0.5GB)
                if vram_gb > 0.5 {
                    let name_len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
                    let name = String::from_utf16_lossy(&desc.Description[..name_len]);
                    let bandwidth = estimate_bandwidth(&name);
                    return Some((name, vram_gb, bandwidth));
                }
            }
            
            adapter_index += 1;
        }
    }
    
    None
}

#[cfg(target_os = "windows")]
fn detect_gpu_nvidia_smi() -> Option<(String, f32, f32)> {
    let output = Command::new("nvidia-smi")
        .args(&["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?;
    let parts: Vec<&str> = line.split(',').collect();
    
    if parts.len() >= 2 {
        let name = parts[0].trim().to_string();
        let vram_mb = parts[1].trim().parse::<f32>().ok()?;
        let vram_gb = vram_mb / 1024.0;
        let bandwidth = estimate_bandwidth(&name);
        Some((name, vram_gb, bandwidth))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn detect_gpu_nvidia_smi() -> Option<(String, f32, f32)> {
    let output = Command::new("nvidia-smi")
        .args(&["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?;
    let parts: Vec<&str> = line.split(',').collect();
    
    if parts.len() >= 2 {
        let name = parts[0].trim().to_string();
        let vram_mb = parts[1].trim().parse::<f32>().ok()?;
        let vram_gb = vram_mb / 1024.0;
        let bandwidth = estimate_bandwidth(&name);
        Some((name, vram_gb, bandwidth))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn detect_gpu_lspci() -> Option<(String, f32, f32)> {
    let output = Command::new("lspci")
        .output()
        .ok()?;
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("VGA compatible controller") || line.contains("3D controller") {
            if let Some(name_start) = line.find(": ") {
                let name = line[name_start + 2..].to_string();
                // Estimate VRAM and bandwidth based on name
                let (vram, bandwidth) = if name.contains("RTX") || name.contains("GTX") {
                    (8.0, 400.0) // Default estimates
                } else if name.contains("Radeon") {
                    (8.0, 400.0)
                } else {
                    continue;
                };
                return Some((name, vram, bandwidth));
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn detect_apple_silicon() -> Option<(String, f32, f32)> {
    let output = Command::new("sysctl")
        .args(&["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()?;
    
    let cpu_brand = String::from_utf8_lossy(&output.stdout).to_string();
    if !cpu_brand.contains("Apple") {
        return None;
    }
    
    let mem_output = Command::new("sysctl")
        .args(&["-n", "hw.memsize"])
        .output()
        .ok()?;
    
    let mem_bytes = String::from_utf8_lossy(&mem_output.stdout)
        .trim()
        .parse::<u64>()
        .ok()?;
    let mem_gb = (mem_bytes as f32) / (1024.0 * 1024.0 * 1024.0);
    
    let (chip, bandwidth) = if cpu_brand.contains("M4") {
        if mem_gb >= 64.0 { ("M4 Max".to_string(), 546.0) }
        else if mem_gb >= 24.0 { ("M4 Pro".to_string(), 273.0) }
        else { ("M4".to_string(), 120.0) }
    } else if cpu_brand.contains("M3") {
        if mem_gb >= 64.0 { ("M3 Max".to_string(), 400.0) }
        else if mem_gb >= 18.0 { ("M3 Pro".to_string(), 150.0) }
        else { ("M3".to_string(), 100.0) }
    } else if cpu_brand.contains("M2") {
        if mem_gb >= 64.0 { ("M2 Ultra".to_string(), 800.0) }
        else if mem_gb >= 32.0 { ("M2 Max".to_string(), 400.0) }
        else if mem_gb >= 16.0 { ("M2 Pro".to_string(), 200.0) }
        else { ("M2".to_string(), 100.0) }
    } else if cpu_brand.contains("M1") {
        if mem_gb >= 64.0 { ("M1 Ultra".to_string(), 800.0) }
        else if mem_gb >= 32.0 { ("M1 Max".to_string(), 400.0) }
        else if mem_gb >= 16.0 { ("M1 Pro".to_string(), 200.0) }
        else { ("M1".to_string(), 68.25) }
    } else {
        ("Apple Silicon".to_string(), 100.0)
    };
    
    Some((chip, mem_gb, bandwidth))
}

fn detect_gpu_info() -> (Option<String>, Option<f32>, Option<f32>) {
    #[cfg(target_os = "windows")]
    {
        // Try DXGI first
        if let Some(result) = detect_gpu_dxgi() {
            return (Some(result.0), Some(result.1), Some(result.2));
        }
        
        // Try nvidia-smi
        if let Some(result) = detect_gpu_nvidia_smi() {
            return (Some(result.0), Some(result.1), Some(result.2));
        }
    }
    
    #[cfg(target_os = "macos")]
    {
        if let Some(result) = detect_apple_silicon() {
            return (Some(result.0), Some(result.1), Some(result.2));
        }
    }
    
    #[cfg(target_os = "linux")]
    {
        // Try nvidia-smi first
        if let Some(result) = detect_gpu_nvidia_smi() {
            return (Some(result.0), Some(result.1), Some(result.2));
        }
        
        // Fallback to lspci
        if let Some(result) = detect_gpu_lspci() {
            return (Some(result.0), Some(result.1), Some(result.2));
        }
    }

    (None, None, None)
}

fn estimate_bandwidth(gpu_name: &str) -> f32 {
    let name_lower = gpu_name.to_lowercase();
    if name_lower.contains("4090") { 1008.0 }
    else if name_lower.contains("4080") { 736.0 }
    else if name_lower.contains("4070") { 504.0 }
    else if name_lower.contains("4060") { 272.0 }
    else if name_lower.contains("3090") { 936.0 }
    else if name_lower.contains("3080") { 760.0 }
    else if name_lower.contains("3070") { 448.0 }
    else if name_lower.contains("3060") { 360.0 }
    else if name_lower.contains("7900 xtx") { 960.0 }
    else if name_lower.contains("7900 xt") { 800.0 }
    else if name_lower.contains("7800") { 624.0 }
    else if name_lower.contains("6900") { 512.0 }
    else if name_lower.contains("6800") { 512.0 }
    else { 300.0 }
}

fn detect_cpu_features() -> (bool, bool) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            let avx512 = is_x86_feature_detected!("avx512f");
            return (true, avx512);
        }
    }
    (false, false)
}

fn detect_storage() -> (Option<f32>, Option<f32>, Option<String>) {
    #[cfg(target_os = "windows")]
    {
        // Get C: drive info
        let output = Command::new("wmic")
            .args(&["logicaldisk", "where", "DeviceID='C:'", "get", "Size,FreeSpace", "/format:csv"])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .output();
        
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines().skip(2) {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 3 {
                    if let (Ok(free), Ok(total)) = (parts[1].trim().parse::<u64>(), parts[2].trim().parse::<u64>()) {
                        let free_gb = (free as f32) / (1024.0 * 1024.0 * 1024.0);
                        let total_gb = (total as f32) / (1024.0 * 1024.0 * 1024.0);
                        return (Some(total_gb), Some(free_gb), Some("Unknown".to_string()));
                    }
                }
            }
        }
    }
    
    #[cfg(target_os = "linux")]
    {
        let output = Command::new("df")
            .args(&["-B1", "/"])
            .output();
        
        if let Ok(output) = output {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = stdout.lines().nth(1) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    if let (Ok(total), Ok(free)) = (parts[1].parse::<u64>(), parts[3].parse::<u64>()) {
                        let total_gb = (total as f32) / (1024.0 * 1024.0 * 1024.0);
                        let free_gb = (free as f32) / (1024.0 * 1024.0 * 1024.0);
                        return (Some(total_gb), Some(free_gb), None);
                    }
                }
            }
        }
    }
    
    (None, None, None)
}

fn main() {
    let mut sys = System::new_all();
    sys.refresh_all();

    // Get CPU info
    let cpu_brand = if let Some(cpu) = sys.cpus().first() {
        cpu.brand().to_string()
    } else {
        "Unknown CPU".to_string()
    };
    
    let cpu_cores = sys.physical_core_count().unwrap_or(4) as u32;
    let cpu_threads = sys.cpus().len() as u32;
    let (cpu_avx2, cpu_avx512) = detect_cpu_features();

    // Get RAM (rounded to whole GB)
    let total_memory = sys.total_memory();
    let ram_gb = ((total_memory as f32) / (1024.0 * 1024.0 * 1024.0)).round() as u32;

    // Get GPU info
    let (gpu_name, vram_gb, bandwidth_gbps) = detect_gpu_info();

    // Get storage
    let (storage_total, storage_free, storage_type) = detect_storage();

    // Get OS
    let os = format!("{} {}", 
        System::name().unwrap_or("Unknown".to_string()),
        System::os_version().unwrap_or("".to_string())
    );

    // Determine if Apple Silicon
    let chip = if cfg!(target_os = "macos") && cpu_brand.contains("Apple") {
        gpu_name.clone()
    } else {
        None
    };

    // Create hardware spec
    let spec = HardwareSpec {
        cpu: cpu_brand.clone(),
        cpu_cores,
        cpu_threads,
        cpu_avx2,
        cpu_avx512,
        ram_gb,
        gpu: if chip.is_some() { None } else { gpu_name.clone() },
        vram_gb: if chip.is_some() { Some(ram_gb as f32) } else { vram_gb },
        os: os.clone(),
        chip: chip.clone(),
        bandwidth_gbps,
        storage_total_gb: storage_total,
        storage_free_gb: storage_free,
        storage_type,
    };

    // Encode spec to base64
    let json = serde_json::to_string(&spec).unwrap();
    let encoded = base64::engine::general_purpose::URL_SAFE.encode(json.as_bytes());

    // Generate URL
    let base_url = option_env!("CANYOURUNLLM_URL").unwrap_or("https://canyourunllm.vercel.app");
    let url = format!("{}/my-pc?spec={}", base_url, encoded);
    
    // Open browser automatically
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(&["/C", "start", "", &url])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .spawn();
    }
}
