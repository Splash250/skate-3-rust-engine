//! Windows managed workers use an AppContainer with no capabilities, a single
//! process Job Object, a handle allowlist and a minimal environment. Native
//! Windows execution must be tested separately from cross-compilation.
use std::{fs::File, io, mem::{size_of, zeroed}, os::windows::{ffi::{OsStrExt,OsStringExt}, io::FromRawHandle}, path::{Path,PathBuf}, ptr::{null,null_mut}, sync::{Arc,Mutex,atomic::{AtomicU64,Ordering}}};
use windows_sys::Win32::{Foundation::*, Security::{*,Authorization::*,Isolation::*}, Storage::FileSystem::*, System::{JobObjects::*,Pipes::CreatePipe,Threading::*,SystemInformation::GetSystemDirectoryW,WindowsProgramming::PROCESS_CREATION_CHILD_PROCESS_RESTRICTED}};

struct Handle(HANDLE);
// SAFETY: handles are owned kernel references, movable across Rust threads; no
// API here mutates Rust memory through their values.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle {fn drop(&mut self){if !self.0.is_null() && self.0!=INVALID_HANDLE_VALUE{unsafe{CloseHandle(self.0);}}}}
impl Handle {fn take(mut self)->HANDLE{let value=self.0;self.0=null_mut();value}}
fn wide(value:&std::ffi::OsStr)->Result<Vec<u16>,String>{let mut value:Vec<_>=value.encode_wide().collect();if value.contains(&0)||value.len()>30_000{return Err("invalid managed worker path".into());}value.push(0);Ok(value)}
fn last(what:&str)->String{format!("managed isolation {what}: {}",io::Error::last_os_error())}
static NEXT:AtomicU64=AtomicU64::new(0);
// ACL read/merge/write must serialize between this host's concurrent resources.
static ACL_LOCK:Mutex<()>=Mutex::new(());

pub(super) struct IsolationGuard {
    job:Arc<Handle>, sid:PSID, profile:Vec<u16>, grants:Vec<PathBuf>,
}
// SAFETY: SID allocation and profile are exclusively owned, ACL edits are locked.
unsafe impl Send for IsolationGuard {}
impl Drop for IsolationGuard {
    fn drop(&mut self){
        unsafe {TerminateJobObject(self.job.0,1);}
        let _lock=ACL_LOCK.lock().unwrap_or_else(|p|p.into_inner());
        for path in self.grants.iter().rev(){let _=edit_acl(path,self.sid,REVOKE_ACCESS);}
        unsafe{DeleteAppContainerProfile(self.profile.as_ptr());FreeSid(self.sid);}
    }
}
pub(super) struct WorkerProcess {
    handle:Handle, job:Arc<Handle>, pid:u32,
    pub(super) stdin:Option<Box<dyn io::Write+Send>>,
    pub(super) stdout:Option<Box<dyn io::Read+Send>>,
}
impl WorkerProcess {
    pub(super) fn id(&self)->u32{self.pid}
    pub(super) fn kill(&mut self)->io::Result<()>{if unsafe{TerminateJobObject(self.job.0,1)}==0{Err(io::Error::last_os_error())}else{Ok(())}}
    pub(super) fn wait(&mut self)->io::Result<()>{if unsafe{WaitForSingleObject(self.handle.0,INFINITE)}==WAIT_FAILED{Err(io::Error::last_os_error())}else{Ok(())}}
}

fn edit_acl(path:&Path,sid:PSID,mode:ACCESS_MODE)->Result<(),String>{
    let path=wide(path.as_os_str())?;
    let mut old=null_mut();let mut descriptor=null_mut();
    // SAFETY: GetNamedSecurityInfo allocates descriptor; old points inside it.
    // SetEntriesInAcl creates a new ACL, both allocations are freed after use.
    unsafe{
        let status=GetNamedSecurityInfoW(path.as_ptr(),SE_FILE_OBJECT,DACL_SECURITY_INFORMATION,null_mut(),null_mut(),&mut old,null_mut(),&mut descriptor);
        if status!=0{return Err(format!("managed runtime ACL read failed ({status})"));}
        if old.is_null(){if !descriptor.is_null(){LocalFree(descriptor);}return Err("managed runtime requires an explicit directory ACL".into());}
        let entry=EXPLICIT_ACCESS_W{grfAccessPermissions:FILE_GENERIC_READ|FILE_GENERIC_EXECUTE,grfAccessMode:mode,grfInheritance:SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            Trustee:TRUSTEE_W{pMultipleTrustee:null_mut(),MultipleTrusteeOperation:NO_MULTIPLE_TRUSTEE,TrusteeForm:TRUSTEE_IS_SID,TrusteeType:TRUSTEE_IS_UNKNOWN,ptstrName:sid.cast()}};
        let mut merged=null_mut();let status=SetEntriesInAclW(1,&entry,old,&mut merged);
        let applied=if status==0 {SetNamedSecurityInfoW(path.as_ptr(),SE_FILE_OBJECT,DACL_SECURITY_INFORMATION,null_mut(),null_mut(),merged,null())}else{status};
        if !merged.is_null(){LocalFree(merged.cast());}if !descriptor.is_null(){LocalFree(descriptor);}
        if applied!=0{return Err(format!("managed runtime ACL update failed ({applied})"));}
    }
    Ok(())
}
struct Attributes {storage:Vec<usize>, pointer:LPPROC_THREAD_ATTRIBUTE_LIST}
impl Attributes {
    fn new()->Result<Self,String>{
        let mut size=0;unsafe{InitializeProcThreadAttributeList(null_mut(),3,0,&mut size);}
        let mut storage=vec![0usize;size.div_ceil(size_of::<usize>())];let pointer=storage.as_mut_ptr().cast();
        if unsafe{InitializeProcThreadAttributeList(pointer,3,0,&mut size)}==0{return Err(last("attribute initialization"));}
        Ok(Self{storage,pointer})
    }
    fn set<T>(&mut self,key:u32,value:&T)->Result<(),String>{
        if unsafe{UpdateProcThreadAttribute(self.pointer,0,key as usize,value as *const _ as *const _,size_of::<T>(),null_mut(),null())}==0{return Err(last("attribute assignment"));}Ok(())
    }
}
impl Drop for Attributes {fn drop(&mut self){unsafe{DeleteProcThreadAttributeList(self.pointer);}let _=&self.storage;}}
fn pipe()->Result<(Handle,Handle),String>{
    let attributes=SECURITY_ATTRIBUTES{nLength:size_of::<SECURITY_ATTRIBUTES>() as u32,lpSecurityDescriptor:null_mut(),bInheritHandle:1};
    let mut read=null_mut();let mut write=null_mut();
    if unsafe{CreatePipe(&mut read,&mut write,&attributes,8192)}==0{return Err(last("pipe creation"));}Ok((Handle(read),Handle(write)))
}
fn parent_only(handle:&Handle)->Result<(),String>{if unsafe{SetHandleInformation(handle.0,HANDLE_FLAG_INHERIT,0)}==0{Err(last("pipe inheritance"))}else{Ok(())}}

pub(super) fn spawn_worker(dotnet:&Path,worker:&Path,memory_bytes:usize)->Result<(WorkerProcess,IsolationGuard),String>{
    let dotnet=dotnet.canonicalize().map_err(|e|e.to_string())?;let worker=worker.canonicalize().map_err(|e|e.to_string())?;
    if !dotnet.join("dotnet.exe").is_file()||!worker.join("Skate.ResourceHost.dll").is_file(){return Err("managed trusted runtime/worker unavailable".into());}
    // Refuse broad ACL changes outside the two configured application trees.
    if dotnet.parent().is_none()||worker.parent().is_none()||dotnet==worker{return Err("managed runtime and worker require separate application directories".into());}
    let serial=NEXT.fetch_add(1,Ordering::Relaxed);
    let name=format!("skate.resource.{}.{}.{}",std::process::id(),serial,std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e|e.to_string())?.as_nanos());
    let profile=wide(std::ffi::OsStr::new(&name))?;
    let mut sid=null_mut();
    if unsafe{CreateAppContainerProfile(profile.as_ptr(),profile.as_ptr(),profile.as_ptr(),null(),0,&mut sid)}<0{return Err(last("AppContainer creation"));}
    let job=unsafe{CreateJobObjectW(null(),null())};
    if job.is_null(){unsafe{DeleteAppContainerProfile(profile.as_ptr());FreeSid(sid);}return Err(last("job creation"));}
    let job=Arc::new(Handle(job));
    let mut guard=IsolationGuard{job:job.clone(),sid,profile,grants:Vec::new()};
    {
        let _lock=ACL_LOCK.lock().unwrap_or_else(|p|p.into_inner());
        for path in [&dotnet,&worker] {edit_acl(path,sid,GRANT_ACCESS)?;guard.grants.push(path.clone());}
    }
    let mut limits:JOBOBJECT_EXTENDED_LIMIT_INFORMATION=unsafe{zeroed()};
    limits.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE|JOB_OBJECT_LIMIT_ACTIVE_PROCESS|JOB_OBJECT_LIMIT_PROCESS_MEMORY|JOB_OBJECT_LIMIT_JOB_MEMORY|JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    limits.BasicLimitInformation.ActiveProcessLimit=1;limits.ProcessMemoryLimit=memory_bytes;limits.JobMemoryLimit=memory_bytes;
    if unsafe{SetInformationJobObject(job.0,JobObjectExtendedLimitInformation,&limits as *const _ as *const _,size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32)}==0{return Err(last("job limits"));}
    let(child_in,parent_in)=pipe()?;let(parent_out,child_out)=pipe()?;
    parent_only(&parent_in)?;parent_only(&parent_out)?;
    let null_name=wide(std::ffi::OsStr::new("NUL"))?;
    let sa=SECURITY_ATTRIBUTES{nLength:size_of::<SECURITY_ATTRIBUTES>() as u32,lpSecurityDescriptor:null_mut(),bInheritHandle:1};
    let stderr=Handle(unsafe{CreateFileW(null_name.as_ptr(),GENERIC_WRITE,FILE_SHARE_READ|FILE_SHARE_WRITE,&sa,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,null_mut())});
    if stderr.0==INVALID_HANDLE_VALUE{return Err(last("stderr setup"));}
    let capabilities=SECURITY_CAPABILITIES{AppContainerSid:sid,Capabilities:null_mut(),CapabilityCount:0,Reserved:0};
    let handles=[child_in.0,child_out.0,stderr.0];
    let child_policy=PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
    let mut attributes=Attributes::new()?;
    attributes.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,&capabilities)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST,&handles)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,&child_policy)?;
    let mut startup:STARTUPINFOEXW=unsafe{zeroed()};startup.StartupInfo.cb=size_of::<STARTUPINFOEXW>() as u32;startup.lpAttributeList=attributes.pointer;
    startup.StartupInfo.dwFlags=STARTF_USESTDHANDLES;startup.StartupInfo.hStdInput=child_in.0;startup.StartupInfo.hStdOutput=child_out.0;startup.StartupInfo.hStdError=stderr.0;
    let executable=dotnet.join("dotnet.exe");let assembly=worker.join("Skate.ResourceHost.dll");
    let executable_wide=wide(executable.as_os_str())?;
    let command=format!("\"{}\" \"{}\"",executable.to_string_lossy(),assembly.to_string_lossy());
    if executable.to_string_lossy().contains('"')||assembly.to_string_lossy().contains('"'){return Err("invalid managed executable path".into());}
    let mut command=wide(std::ffi::OsStr::new(&command))?;
    let directory=wide(worker.as_os_str())?;
    let mut system=[0u16;32768];let count=unsafe{GetSystemDirectoryW(system.as_mut_ptr(),system.len() as u32)} as usize;
    if count==0||count>=system.len(){return Err(last("system directory"));}
    let system=PathBuf::from(std::ffi::OsString::from_wide(&system[..count]));
    let system_root=system.parent().ok_or("Windows system directory invalid")?;
    let mut environment=std::collections::BTreeMap::from([
        ("DOTNET_ROOT",dotnet.to_string_lossy().into_owned()),("DOTNET_EnableDiagnostics","0".into()),
        ("DOTNET_CLI_TELEMETRY_OPTOUT","1".into()),("DOTNET_SYSTEM_GLOBALIZATION_INVARIANT","1".into()),("DOTNET_PROCESSOR_COUNT","2".into()),
        ("DOTNET_GCHeapHardLimit",format!("{:x}",memory_bytes/2)),("SystemRoot",system_root.to_string_lossy().into_owned())]);
    let mut block=Vec::new();for(key,value)in &mut environment{block.extend(wide(std::ffi::OsStr::new(&format!("{key}={value}")))?);}block.push(0);
    let mut info:PROCESS_INFORMATION=unsafe{zeroed()};
    let created=unsafe{CreateProcessW(executable_wide.as_ptr(),command.as_mut_ptr(),null(),null(),1,CREATE_SUSPENDED|CREATE_NO_WINDOW|CREATE_UNICODE_ENVIRONMENT|EXTENDED_STARTUPINFO_PRESENT,block.as_ptr().cast(),directory.as_ptr(),&startup.StartupInfo,&mut info)};
    if created==0{return Err(last("AppContainer process creation"));}
    let process=Handle(info.hProcess);let thread=Handle(info.hThread);
    if unsafe{AssignProcessToJobObject(job.0,process.0)}==0{unsafe{TerminateProcess(process.0,1);WaitForSingleObject(process.0,INFINITE);}return Err(last("job assignment"));}
    if unsafe{ResumeThread(thread.0)}==u32::MAX{unsafe{TerminateJobObject(job.0,1);WaitForSingleObject(process.0,INFINITE);}return Err(last("worker resume"));}
    // SAFETY: exclusively owned parent pipe handles are transferred into Files.
    let stdin=unsafe{File::from_raw_handle(parent_in.take())};let stdout=unsafe{File::from_raw_handle(parent_out.take())};
    Ok((WorkerProcess{handle:process,job,pid:info.dwProcessId,stdin:Some(Box::new(stdin)),stdout:Some(Box::new(stdout))},guard))
}
