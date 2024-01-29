use std::ffi::c_void;
use std::mem;
use std::str::from_utf8_unchecked;

use encoding_rs::SHIFT_JIS;
use windows::Win32::Foundation::ERROR_PARTIAL_COPY;
use windows::Win32::Foundation::GetLastError;
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Memory::MEMORY_BASIC_INFORMATION;
use windows::Win32::System::Memory::VirtualQueryEx;
use windows::Win32::System::ProcessStatus::PSAPI_WORKING_SET_EX_BLOCK;
use windows::Win32::System::ProcessStatus::PSAPI_WORKING_SET_EX_INFORMATION;
use windows::Win32::System::ProcessStatus::QueryWorkingSetEx;
use windows::Win32::{System::{Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32, TH32CS_SNAPPROCESS, Process32Next}, Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ, GetExitCodeProcess}}, Foundation::{STILL_ACTIVE, HANDLE, CloseHandle}};

const VALID_PROCESS_NAMES: &'static [&'static str] = &["Dolphin.exe", "Slippi Dolphin.exe", "Slippi_Dolphin.exe", "DolphinWx.exe", "DolphinQt2.exe"];
const GC_RAM_START: u32 = 0x80000000;
const GC_RAM_END: u32 = 0x81800000;
const GC_RAM_SIZE: usize = 0x2000000;
const MEM_MAPPED: u32 = 0x40000;

pub struct DolphinMemory {
    process_handle: Option<HANDLE>,
    dolphin_base_addr: Option<*mut c_void>,
    dolphin_addr_size: Option<usize>
}

impl DolphinMemory {
    pub fn new() -> Self {
        DolphinMemory { process_handle: None, dolphin_base_addr: None, dolphin_addr_size: None }
    }

    pub fn find_process(&mut self) -> bool {
        unsafe {
            let mut status: u32 = 0;
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).unwrap();
            let mut pe32 = PROCESSENTRY32 {
                dwSize: mem::size_of::<PROCESSENTRY32>() as u32,
                cntUsage: 0,
                th32ProcessID: 0,
                th32DefaultHeapID: 0,
                th32ModuleID: 0,
                cntThreads: 0,
                th32ParentProcessID: 0,
                pcPriClassBase: 0,
                dwFlags: 0,
                szExeFile: [0; 260]
            };

            loop {
                if !Process32Next(snapshot, &mut pe32 as *mut _).as_bool() {
                    break;
                }
                let name = from_utf8_unchecked(&pe32.szExeFile);
                if VALID_PROCESS_NAMES.iter().any(|&e| name.starts_with(e)) {
                    println!("{}", name);
                    let handle_res = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pe32.th32ProcessID);
                    if handle_res.is_ok() {
                        let handle = handle_res.unwrap();
                        if GetExitCodeProcess(handle, &mut status as *mut _).as_bool() && status as i32 == STILL_ACTIVE.0 {
                            self.process_handle = Some(handle);
                            break;
                        }
                    } else {
                        // ? handle is supposed to be null so what will be closed... ported from m-overlay, see reference on the top
                        CloseHandle(handle_res.unwrap());
                        self.process_handle = None;
                    }
                } else {
                    self.process_handle = None;
                }
            }
            CloseHandle(snapshot);
            return self.has_process();
        }
    }

    pub fn has_process(&self) -> bool {
        self.process_handle.is_some()
    }

    pub fn check_process_running(&mut self) -> bool {
        if self.process_handle.is_none() {
            return false;
        }

        let mut status: u32 = 0;
        unsafe {
            if GetExitCodeProcess(self.process_handle.unwrap(), &mut status as *mut _).as_bool() && status as i32 != STILL_ACTIVE.0 {
                self.reset();
                return false;
            }
        }
        return true;
    }

    pub fn read<T: Sized>(&mut self, addr: u32) -> Option<T> where [u8; mem::size_of::<T>()]:{
        if !self.has_process() || (!self.has_gamecube_ram_offset() && !self.find_gamecube_ram_offset()) {
            return None;
        }

        let mut addr = addr;
        if addr >= GC_RAM_START && addr <= GC_RAM_END {
            addr -= GC_RAM_START;
        } else {
            println!("[MEMORY] Attempt to read from invalid address {:#08x}", addr);
            return None;
        }

        let raddr = self.dolphin_base_addr.unwrap() as usize + addr as usize;
        let mut output = [0u8; mem::size_of::<T>()];
        let size = mem::size_of::<T>();
        let mut memread: usize = 0;
        
        unsafe {
            let success = ReadProcessMemory(self.process_handle.unwrap(), raddr as *const c_void, &mut output as *mut _ as *mut c_void, size, Some(&mut memread as *mut _));
            if success.as_bool() && memread == size {
                // because win32 decides to give me the output in the wrong endianness, we'll reverse it
                output.reverse(); // TODO figure out if we really have to do this, i would like to avoid it if possible
                return Some(mem::transmute_copy(&output));
            } else {
                let err = GetLastError().0;
                println!("[MEMORY] Failed reading from address {:#08X} ERROR {}", addr, err);
                if err == ERROR_PARTIAL_COPY.0 { // game probably closed, reset the dolphin ram offset
                    self.dolphin_addr_size = None;
                    self.dolphin_base_addr = None;
                }
                return None;
            }
        }
    }

    pub fn read_string<const LEN: usize>(&mut self, addr: u32) -> Option<String> where [(); mem::size_of::<[u8; LEN]>()]:{
        let res = self.read::<[u8; LEN]>(addr);
        if res.is_none() {
            return None;
        }

        let mut raw = res.unwrap();
        raw.reverse(); // we apparently have to reverse it again due to how the string is gathered

        return match std::str::from_utf8(&raw) {
            Ok(v) => Some(v.trim_end_matches(char::from(0)).into()),
            Err(e) => {
                println!("Invalid utf-8 string => {:?} | {}", res.unwrap(), e.to_string());
                None
            }
        };
    }

    pub fn read_string_shift_jis<const LEN: usize>(&mut self, addr: u32) -> Option<String> where [(); mem::size_of::<[u8; LEN]>()]:{
        let res = self.read::<[u8; LEN]>(addr);
        if res.is_none() {
            return None;
        }

        let mut raw = res.unwrap();
        raw.reverse(); // we apparently have to reverse it again due to how the string is gathered

        let (dec_res, _enc, errors) = SHIFT_JIS.decode(&raw);
        if errors {
            println!("Invalid shift-jis string => {:?}", res.unwrap())
        }
        return Some(dec_res.as_ref().trim_end_matches(char::from(0)).to_string());
    }

    pub fn pointer_indirection(&mut self, addr: u32, amount: u32) -> Option<u32> {
        let mut curr = self.read::<u32>(addr);
        for n in 2..=amount {
            if curr.is_none() {
                return None;
            }
            curr = self.read::<u32>(curr.unwrap());
        }
        curr
    }

    /*pub fn write(&self) {

    }*/

    fn find_gamecube_ram_offset(&mut self) -> bool {
        if !self.has_process() {
            return false;
        }

        unsafe {
            let mut info: MEMORY_BASIC_INFORMATION = Default::default();
            let mut address: usize = 0;

            while VirtualQueryEx(self.process_handle.unwrap(), Some(address as *const c_void), &mut info as *mut _, mem::size_of::<MEMORY_BASIC_INFORMATION>()) == mem::size_of::<MEMORY_BASIC_INFORMATION>() {
                address = address + info.RegionSize / mem::size_of::<usize>();
                // Dolphin stores the GameCube RAM address space in 32MB chunks.
		        // Extended memory override can allow up to 64MB.
                if info.RegionSize >= GC_RAM_SIZE && info.RegionSize % GC_RAM_SIZE == 0 && info.Type.0 == MEM_MAPPED {
                    let mut wsinfo = PSAPI_WORKING_SET_EX_INFORMATION {
                        VirtualAddress: 0 as *mut c_void,
                        VirtualAttributes: PSAPI_WORKING_SET_EX_BLOCK { Flags: 0 }
                    };
                    wsinfo.VirtualAddress = info.BaseAddress;

                    if QueryWorkingSetEx(self.process_handle.unwrap(), &mut wsinfo as *mut _ as *mut c_void, mem::size_of::<PSAPI_WORKING_SET_EX_INFORMATION>().try_into().unwrap()).as_bool() {
                        if (wsinfo.VirtualAttributes.Flags & 1) == 1 && info.BaseAddress != 0 as *mut c_void {
                            self.dolphin_base_addr = Some(info.BaseAddress);
                            self.dolphin_addr_size = Some(info.RegionSize);

                            println!("Dolphin Base Address: {:?}", self.dolphin_base_addr);
                            println!("Dolphin Address Size: {:?}", self.dolphin_addr_size);
                            return true;
                        }
                    }
                }
            }
        }

        return false;
    }

    fn has_gamecube_ram_offset(&self) -> bool {
        self.dolphin_base_addr.is_some()
    }

    fn reset(&mut self) {
        self.process_handle = None;
        self.dolphin_base_addr = None;
        self.dolphin_addr_size = None;
    }
}

pub mod util {
    macro_rules! R13 {($offset:expr) => { 0x804db6a0 - $offset }}
    pub(crate) use R13;
}
