// Dedicated local shells keep adversarial browser input away from the live demo.
import {spawn} from 'node:child_process';
import {open,mkdtemp,readFile,writeFile,rm,mkdir} from 'node:fs/promises';
import {createServer} from 'node:net';
import {randomUUID} from 'node:crypto';
import path from 'node:path';

const required=name=>{const value=process.env[name];if(!value||!path.isAbsolute(value))throw new Error(`Set ${name} to the absolute Bazel-built artifact path.`);return value;};
const host=required('TERMINAL_TEST_HOST'),holder=required('TERMINAL_TEST_HOLDER'),bundle=required('TERMINAL_TEST_BUNDLE');
const smoke=process.argv[2];
const protocol=Number(process.env.TERMINAL_TEST_PROTOCOL || 3);
if(!Number.isInteger(protocol)||protocol<1)throw new Error('Set TERMINAL_TEST_PROTOCOL to the version of the Bazel-built fixture host.');
const sshConfig=process.env.TERMINAL_TEST_SSH_CONFIG;
if(sshConfig&&!path.isAbsolute(sshConfig))throw new Error('Set TERMINAL_TEST_SSH_CONFIG to an absolute trusted Lima SSH configuration path.');
const remoteRoot=`/var/tmp/h2-browser-${randomUUID().replaceAll('-','').slice(0,12)}`;
if(!smoke||typeof WebSocket!=='function')throw new Error('Run fixture_runtime_test through Bazel with Node22+ selected by TERMINAL_LAB_NODE.');
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function within(promise,ms,message,onTimeout){let timer;try{return await Promise.race([promise,new Promise((_,reject)=>{timer=setTimeout(()=>{onTimeout?.();reject(new Error(message));},ms);})]);}finally{clearTimeout(timer);}}
// Unix-domain sockets need a short path even when Bazel's test root is long.
const root=await mkdtemp('/tmp/h2-bf-');
const log=await open(path.join(root,'host.log'),'w',0o600);
let server,viewer,hostError,interrupted=false,token,port,allStopped=false;
const childExit=child=>new Promise((resolve,reject)=>{if(child.exitCode!==null||child.signalCode!==null)resolve(child.exitCode);else{child.once('exit',resolve);child.once('error',reject);}});
for(const signal of ['SIGTERM','SIGINT'])process.on(signal,()=>{interrupted=true;viewer?.kill('SIGTERM');});

async function freePort(){const listener=createServer();await new Promise((resolve,reject)=>{listener.once('error',reject);listener.listen(0,'127.0.0.1',resolve);});const port=listener.address().port;await new Promise(resolve=>listener.close(resolve));return port;}
async function stopShells(){
 if(!token||!server||server.exitCode!==null||server.signalCode!==null)return false;
 const socket=new WebSocket(`ws://127.0.0.1:${port}/ws`);
 const stopped=new Set();let authenticated=false,failed;
 const send=frame=>socket.send(JSON.stringify(frame));
 const done=new Promise((resolve,reject)=>{
  socket.addEventListener('error',()=>reject(new Error('Could not connect to stop the dedicated fixture shells.')),{once:true});
  socket.addEventListener('open',()=>send({type:'authenticate',protocol,token}),{once:true});
  socket.addEventListener('message',event=>{
   try{
    const frame=JSON.parse(event.data);
    if(frame.type==='sessions'){
     if(!authenticated){authenticated=true;for(const session of frame.sessions){if(session.state.state!=='running')stopped.add(session.id);else send({type:'open',id:session.id});}}
    }else if(frame.type==='control'){
     if(frame.controller)send({type:'stop',id:frame.id});else send({type:'take_control',id:frame.id});
    }else if(frame.type==='exited')stopped.add(frame.id);
    else if(frame.type==='error'){failed=new Error(`Could not stop fixture terminal ${frame.id}: ${frame.code}. Private state was preserved.`);reject(failed);}
    if(stopped.size===3)resolve(true);
   }catch(error){reject(error);}
  });
 });
 try{return await within(done,10000,'Timed out stopping dedicated fixture shells; private state was preserved.');}
 finally{socket.close();}
}

try{
 port=await freePort();
 const profiles=[1,2,3].map(id=>({name:`shell${id}`,title:`Fixture shell ${id}`,program:'/usr/bin/env',args:['HISTFILE=/dev/null','/bin/bash','--noprofile','--norc','-i'],cwd:root,endpoint:'Local',recovery:null,history:{mode:'recent',max_lines:10000,max_bytes:4<<20,interval_seconds:30}}));
 if(sshConfig)profiles[1]={...profiles[1],title:'Fixture SSH Debian',program:'/usr/bin/env',args:['HISTFILE=/dev/null','/bin/bash','--noprofile','--norc','-i'],cwd:'/var/tmp',endpoint:{Ssh:{config:sshConfig,destination:'lima-h2-terminal-c1',holder:'/tmp/h2-terminal-holder',root:remoteRoot}}};
 await writeFile(path.join(root,'profiles.json'),JSON.stringify(profiles),{mode:0o600});
 // Files are deliberate Tab-completion fixtures, never files in the user's repo.
 await writeFile(path.join(root,'completion-fixture.txt'),'terminal completion acceptance\n',{mode:0o600});
 server=spawn(host,['--state',root,'--holder',holder,'--static-dir',bundle,'--listen',`127.0.0.1:${port}`,'--profiles',path.join(root,'profiles.json'),...profiles.flatMap(profile=>['--start',profile.name])],{stdio:['ignore',log.fd,log.fd]});
 server.once('error',error=>{hostError=error;});
 for(let attempt=0;attempt<300;attempt++){
  if(interrupted)throw new Error('Fixture acceptance was interrupted.');
  if(hostError)throw new Error(`Could not start the Bazel-built host at ${host}. Rebuild the host target.`,{cause:hostError});
  if(server.exitCode!==null||server.signalCode!==null)throw new Error(`The dedicated host exited. Inspect ${path.join(root,'host.log')}.`);
  try{if((await fetch(`http://127.0.0.1:${port}/health`,{signal:AbortSignal.timeout(1000)})).ok){token=(await readFile(path.join(root,'browser.token'),'utf8')).trim();break;}}catch{}
  await sleep(100);
 }
 if(!token)throw new Error(`The dedicated host did not become healthy. Inspect ${path.join(root,'host.log')}.`);
 if(process.env.TERMINAL_TEST_CLI==='1'){
  if(!process.argv[4])throw new Error('Run the CLI proof through the Bazel fixture target.');
  await writeFile(path.join(root,'server.json'),JSON.stringify({pid:server.pid,port}),{mode:0o600});
  const saved=JSON.parse(await readFile(path.join(root,'workspace.json'),'utf8'));
  const uuid=saved.sessions[0].uuid,nonce=randomUUID().replaceAll('-','').slice(0,12),input=path.join(root,'cli-input');
  await writeFile(input,`printf 'CLI_${nonce}_%s\\n' READY\r`,{mode:0o600});
  viewer=spawn(process.execPath,[process.argv[4],'--state',root,'--uuid',uuid,'--file',input,'--expect',`CLI_${nonce}_READY`],{stdio:'inherit'});
  const status=await childExit(viewer);
  if(status!==0||interrupted)throw new Error(`UUID-addressed CLI injection failed (${status??'signal'}).`);
 }
 const urlFile=path.join(root,'viewer.url');
 await writeFile(urlFile,`http://127.0.0.1:${port}/${process.env.TERMINAL_BROWSER_MEMORY==='1'?'?memory=1':''}#token=${token}\n`,{mode:0o600});
 viewer=spawn(process.execPath,[smoke],{stdio:'inherit',env:{...process.env,TERMINAL_VIEWER_URL:'',TERMINAL_VIEWER_URL_FILE:urlFile,TERMINAL_BROWSER_INPUT:'1',TERMINAL_BROWSER_CLIPBOARD:'0',TERMINAL_EXPECT_SESSION_OS:JSON.stringify({1:process.platform==='darwin'?'Darwin':'Linux',2:sshConfig?'Linux':process.platform==='darwin'?'Darwin':'Linux'})}});
 const status=await childExit(viewer);
 if(status!==0||interrupted)throw new Error(`Dedicated browser acceptance failed (${status??'signal'}). Inspect the Bazel browser artifacts.`);
 if(process.env.TERMINAL_TEST_LINUX_PROOF==='1'){
  if(!sshConfig||!process.argv[3])throw new Error('The Linux proof needs the dedicated SSH fixture and Bazel-provided Linux runner.');
  const linuxUrl=new URL((await readFile(urlFile,'utf8')).trim());linuxUrl.searchParams.delete('memory');
  const linuxUrlFile=path.join(root,'linux.url');await writeFile(linuxUrlFile,linuxUrl.href+'\n',{mode:0o600});
  const output=path.join(process.env.TEST_UNDECLARED_OUTPUTS_DIR||root,'linux');await mkdir(output,{recursive:true});
  viewer=spawn('/bin/sh',[process.argv[3]],{stdio:'inherit',env:{...process.env,TEST_UNDECLARED_OUTPUTS_DIR:output,TERMINAL_LAB_SSH_CONFIG:sshConfig,TERMINAL_VIEWER_URL_FILE:linuxUrlFile,TERMINAL_EXPECT_SESSION_OS:JSON.stringify({1:'Darwin',2:'Linux'}),TERMINAL_BROWSER_MIRROR:'0',TERMINAL_BROWSER_MEMORY:'0',TERMINAL_BROWSER_FLAGS:process.env.TERMINAL_TEST_LINUX_FLAGS||'["--no-sandbox","--no-zygote","--renderer-process-limit=1","--use-gl=angle","--use-angle=swiftshader-webgl","--enable-unsafe-swiftshader","--disable-features=Vulkan"]'}});
  const status=await childExit(viewer);
  if(status!==0||interrupted)throw new Error(`Linux browser acceptance failed (${status??'signal'}). Inspect the Linux Bazel artifacts.`);
 }
}finally{
 try{allStopped=await stopShells();}catch(error){console.error(error.message);}
 if(server&&server.exitCode===null&&server.signalCode===null){server.kill('SIGTERM');try{await within(childExit(server),4000,'Fixture host shutdown timed out.');}catch{server.kill('SIGKILL');await childExit(server);}}
 await log.close();
 if(process.env.TEST_UNDECLARED_OUTPUTS_DIR){const text=(await readFile(path.join(root,'host.log'),'utf8')).slice(-128*1024);await writeFile(path.join(process.env.TEST_UNDECLARED_OUTPUTS_DIR,'fixture-host.log'),token?text.split(token).join('[redacted]'):text);}
 if(allStopped){
  if(sshConfig){const cleanup=spawn('/usr/bin/ssh',['-F',sshConfig,'-o','BatchMode=yes','-o','ConnectTimeout=8','-o','ControlMaster=no','-o','ControlPath=none','lima-h2-terminal-c1',`rm -rf -- '${remoteRoot}'`],{stdio:['ignore','ignore','inherit']});let status;try{status=await within(childExit(cleanup),12000,'Remote fixture cleanup timed out.',()=>cleanup.kill('SIGTERM'));}catch{status=1;}if(status!==0){console.error(`Stopped SSH shell, but could not remove its owned temporary directory ${remoteRoot}.`);process.exitCode=1;}}
  await rm(root,{recursive:true,force:true});console.log('Dedicated fixture stopped all three shells and its host; the live demo was untouched.');
 }
 else{console.error(`Fixture cleanup needs attention; private state retained at ${root}. The live demo was untouched.`);process.exitCode=1;}
}
