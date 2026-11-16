// Manual real-browser acceptance. No page-side terminal emulator or process access.
import {spawn} from 'node:child_process';
import {randomUUID} from 'node:crypto';
import {inflateSync} from 'node:zlib';
import {mkdtemp,readFile,rm,writeFile,mkdir} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';

const nonce=randomUUID().replaceAll('-','').slice(0,12);
const expectedSessionOs=JSON.parse(process.env.TERMINAL_EXPECT_SESSION_OS || '{}');
const outputLines=terminal=>terminal.text.split('\n').map(line=>line.trim());
const visibleTerminals=state=>(state.terminals||[]).filter(terminal=>state.selected?.includes(terminal.id));
const selectedReady=state=>state?.connected&&state.selected?.some(id=>id!==null)&&state.selected.filter(id=>id!==null).every(id=>state.terminals?.some(t=>t.id===id&&t.viewport?.width>0&&t.text.trim().length));
const commandOs=(terminal,marker)=>{const lines=outputLines(terminal),index=lines.indexOf(marker);return index>=0?lines[index+1]:undefined;};
const controlPoint=terminal=>{const b=terminal.take_control;if(!b||b.width<=0||b.height<=0)throw new Error(`Take control button geometry is unavailable for terminal ${terminal.id}.`);return {x:b.x+b.width/2,y:b.y+b.height/2};};
let url=process.env.TERMINAL_VIEWER_URL || (process.env.TERMINAL_VIEWER_URL_FILE ? (await readFile(process.env.TERMINAL_VIEWER_URL_FILE,'utf8')).trim() : undefined);
if(!url)throw new Error('Set TERMINAL_VIEWER_URL or TERMINAL_VIEWER_URL_FILE to the running authenticated terminal viewer URL.');
if(process.env.TERMINAL_BROWSER_MEMORY==='1'){const configured=new URL(url);configured.searchParams.set('memory','1');url=configured.href;}
const capability=new URLSearchParams(new URL(url).hash.replace(/^#/, '')).get('token');
const redact=text=>capability?String(text).split(capability).join('[redacted]'):String(text);
const jsonArtifact=value=>redact(JSON.stringify(value,null,2));
if(typeof WebSocket!=='function')throw new Error('This manual browser test needs Node 22 or newer with built-in WebSocket.');
if(process.env.TERMINAL_BROWSER_CLIPBOARD==='1'&&(process.platform!=='linux'||process.env.TERMINAL_BROWSER_ISOLATED_CLIPBOARD!=='1'))throw new Error('Clipboard acceptance requires an isolated Linux browser VM and TERMINAL_BROWSER_ISOLATED_CLIPBOARD=1. It never reads or changes the Mac user clipboard.');
const chrome=process.env.TERMINAL_BROWSER || (process.platform==='darwin'?'/Applications/Google Chrome.app/Contents/MacOS/Google Chrome':'/usr/bin/chromium');
const output=process.env.TEST_UNDECLARED_OUTPUTS_DIR || process.env.TERMINAL_BROWSER_OUTPUT || '/tmp/terminal-browser-proof';
await mkdir(output,{recursive:true});
const profile=await mkdtemp(path.join(tmpdir(),'terminal-browser-'));
const flags=['--headless=new','--remote-debugging-port=0',`--user-data-dir=${profile}`,'--no-first-run','--no-default-browser-check','--disable-background-networking','--disable-component-update','--disable-sync','--disable-background-timer-throttling','--disable-renderer-backgrounding','--disable-backgrounding-occluded-windows','--enable-unsafe-webgpu','--enable-unsafe-swiftshader','--disable-dev-shm-usage','--hide-scrollbars','about:blank'];
if(process.platform==='darwin')flags.push('--use-mock-keychain','--password-store=basic');
if(process.env.TERMINAL_BROWSER_FLAGS)flags.unshift(...JSON.parse(process.env.TERMINAL_BROWSER_FLAGS));
const child=spawn(chrome,flags,{detached:process.platform!=='win32',stdio:['ignore','ignore','pipe']});
let stderr='';child.stderr.on('data',chunk=>{stderr=(stderr+chunk).slice(-12000);});
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
function pngMetrics(bytes,region){
 if(bytes.subarray(0,8).toString('hex')!=='89504e470d0a1a0a')throw new Error('Chrome returned an invalid PNG screenshot.');
 let width,height,channels;const chunks=[];
 for(let pos=8;pos<bytes.length;){const size=bytes.readUInt32BE(pos),kind=bytes.toString('ascii',pos+4,pos+8),body=bytes.subarray(pos+8,pos+8+size);if(kind==='IHDR'){width=body.readUInt32BE(0);height=body.readUInt32BE(4);channels=body[9]===2?3:body[9]===6?4:0;if(body[8]!==8||!channels||body[12]!==0)throw new Error('Screenshot verification needs an 8-bit RGB/RGBA noninterlaced PNG.');}if(kind==='IDAT')chunks.push(body);pos+=12+size;}
 const stride=width*channels,raw=inflateSync(Buffer.concat(chunks));if(raw.length!==(stride+1)*height)throw new Error('Chrome screenshot PNG has inconsistent dimensions.');
 let prior=Buffer.alloc(stride),at=0,nonDark=0,redPixels=0;
 for(let y=0;y<height;y++){
  const filter=raw[at++],row=Buffer.alloc(stride);if(filter>4)throw new Error('Screenshot PNG uses an unknown scanline filter.');
  for(let x=0;x<stride;x++){const left=x>=channels?row[x-channels]:0,up=prior[x],corner=x>=channels?prior[x-channels]:0;let predictor=0;if(filter===1)predictor=left;else if(filter===2)predictor=up;else if(filter===3)predictor=Math.floor((left+up)/2);else if(filter===4){const p=left+up-corner,a=Math.abs(p-left),b=Math.abs(p-up),c=Math.abs(p-corner);predictor=a<=b&&a<=c?left:b<=c?up:corner;}row[x]=(raw[at++]+predictor)&255;}
  for(let x=0;x<stride;x+=channels){if(region&&(x/channels<region.x||x/channels>=region.x+region.width||y<region.y||y>=region.y+region.height))continue;const r=row[x],g=row[x+1],b=row[x+2];if(Math.max(r,g,b)>64)nonDark++;if(Math.abs(r-235)<18&&Math.abs(g-76)<18&&Math.abs(b-88)<18)redPixels++;}
  prior=row;
 }
 return {width,height,nonDarkPixels:nonDark,redProbePixels:redPixels};
}
function requirePaint(metrics){if(metrics.nonDarkPixels<Math.max(1000,metrics.width*metrics.height*.001))throw new Error(`The actual compositor screenshot has no visible terminal/UI paint (${metrics.nonDarkPixels} lit pixels). Model state alone does not certify rendering.`);}

let cleanupPromise;
function chromeAlive(){return child.exitCode===null&&child.signalCode===null;}
function stopOwnedChrome(signal){
 if(!chromeAlive())return;
 try{if(process.platform!=='win32'&&child.pid)process.kill(-child.pid,signal);else child.kill(signal);}catch(error){if(error.code!=='ESRCH')throw error;}
}
function cleanup(){
 return cleanupPromise ||= (async()=>{
  const exited=new Promise(resolve=>{if(!chromeAlive())resolve();else child.once('exit',resolve);});
  stopOwnedChrome('SIGTERM');await Promise.race([exited,sleep(1500)]);
  if(chromeAlive()){stopOwnedChrome('SIGKILL');await Promise.race([exited,sleep(500)]);}
  await rm(profile,{recursive:true,force:true,maxRetries:3,retryDelay:100});
 })();
}
let terminating=false;
for(const [signal,code] of [['SIGINT',130],['SIGTERM',143]])process.once(signal,()=>{
 if(terminating)return;terminating=true;
 cleanup().then(()=>process.exit(code),error=>{console.error(`Could not clean up the temporary browser: ${error}`);process.exit(code);});
});

class CDP {
 constructor(socket){this.socket=socket;this.next=1;this.waiting=new Map();this.events=[];this.network=[];this.firstNetwork=[];socket.addEventListener('message',event=>{const message=JSON.parse(event.data);if(message.id){const pending=this.waiting.get(message.id);if(!pending)return;this.waiting.delete(message.id);message.error?pending.reject(new Error(message.error.message)):pending.resolve(message.result);}else{if(['Network.webSocketFrameSent','Network.webSocketFrameReceived'].includes(message.method)){
 const frame=message.params.response,payload=frame.payloadData||'',head=payload.slice(0,1024);
 const field=name=>head.match(new RegExp('"'+name+'":([0-9]+)'))?.[1];
 const kind=head.match(/^\{"type":"([^"]+)"/)?.[1]||'unparsed';
 const code=head.match(/"code":"([^"]{0,128})"/)?.[1];
 message.params={requestId:message.params.requestId,timestamp:message.params.timestamp,summary:{direction:message.method.endsWith('Sent')?'sent':'received',type:kind,id:field('id'),seq:field('seq'),length:payload.length,opcode:frame.opcode,errorCode:code,closeCode:frame.opcode===8&&payload.length?(()=>{const bytes=Buffer.from(payload,'base64');return bytes.length>=2?bytes.readUInt16BE(0):undefined;})():undefined}};
 }if(message.method==='Runtime.consoleAPICalled'){
 const params=message.params;message.params={type:params.type,timestamp:params.timestamp,args:params.args.map(arg=>({type:arg.type,value:typeof arg.value==='string'?arg.value.slice(0,4096):typeof arg.value==='number'||typeof arg.value==='boolean'?arg.value:undefined,description:arg.description?.slice(0,4096)})),stackTrace:params.stackTrace?{callFrames:params.stackTrace.callFrames?.slice(0,8)}:undefined};
 }if(message.method.startsWith('Network.webSocket')){if(this.firstNetwork.length<32)this.firstNetwork.push(message);this.network.push(message);if(this.network.length>200)this.network.shift();}if(!this.firstFailure&&(message.params?.summary?.type==='error'||message.method==='Network.webSocketFrameError'))this.firstFailure=message;this.events.push(message);if(this.events.length>500)this.events.shift();}});}
 async send(method,params={},sessionId,timeoutMs=15000){
  const id=this.next++;let timer;
  const result=new Promise((resolve,reject)=>{
   this.waiting.set(id,{resolve,reject});
   timer=setTimeout(()=>{this.waiting.delete(id);reject(new Error(`Chrome did not answer ${method} within ${timeoutMs}ms`));},timeoutMs);
  });
  try{this.socket.send(JSON.stringify({id,method,params,sessionId}));return await result;}finally{clearTimeout(timer);this.waiting.delete(id);}
 }
}
let cdp,activeCall,activeMirrorCall,reloadBootstrap;
const bootstrap={phase:"chrome-startup",started:Date.now(),budgetMs:60000};
const inspectExpression="window.terminalViewer?.phase==='failed' ? ({phase:window.terminalViewer.phase,errors:window.terminalViewer.errors}) : (window.terminalInspect ? JSON.parse(window.terminalInspect()) : ({phase:window.terminalViewer?.phase,errors:window.terminalViewer?.errors}))";
function checkedState(state){if(state?.phase==='failed')throw new Error(`Browser GPUI runtime failed: ${JSON.stringify(state.errors)}`);return state;}
try{
 let port;
 for(let i=0;i<200;i++){try{port=(await readFile(path.join(profile,'DevToolsActivePort'),'utf8')).trim().split('\n');break;}catch{}if(child.exitCode!==null)throw new Error(`Headless Chrome exited (${child.exitCode}): ${stderr}`);await sleep(50);}
 if(!port)throw new Error(`Headless Chrome did not start: ${stderr}`);
 const socket=new WebSocket(`ws://127.0.0.1:${port[0]}${port[1]}`);await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});cdp=new CDP(socket);
 const target=await cdp.send('Target.createTarget',{url:'about:blank'});const attached=await cdp.send('Target.attachToTarget',{targetId:target.targetId,flatten:true});const session=attached.sessionId;
 const call=(method,params={})=>cdp.send(method,params,session);activeCall=call;
 await call('Page.enable');await call('Runtime.enable');await call('Log.enable');await call('Network.enable');await call('Emulation.setDeviceMetricsOverride',{width:1280,height:800,deviceScaleFactor:1,mobile:false});
 await cdp.send('Browser.grantPermissions',{origin:new URL(url).origin,permissions:['clipboardReadWrite','clipboardSanitizedWrite']});
 await call('Page.addScriptToEvaluateOnNewDocument',{source:`(()=>{
  const events=[];window.terminalFocusMetadata=()=>({hasFocus:document.hasFocus(),activeTag:document.activeElement?.tagName??null,visibility:document.visibilityState,events:events.slice()});
  for(const type of ['focusin','focusout','pointerdown','pointerup','keydown','keyup'])document.addEventListener(type,event=>{
   const key=['KeyF','KeyV','KeyC','Enter','Tab','Escape'].includes(event.code)?event.code:event.code?'other':undefined;
   events.push({time:performance.now(),type,targetTag:event.target?.tagName??null,activeTag:document.activeElement?.tagName??null,hasFocus:document.hasFocus(),key,ctrl:event.ctrlKey,shift:event.shiftKey,alt:event.altKey,meta:event.metaKey,x:event.clientX,y:event.clientY});if(events.length>64)events.shift();
  },true);
 })()`});
 await call('Page.navigate',{url});
 await call('Page.bringToFront');
 const evaluate=async(expression,timeoutMs=15000)=>{const value=await cdp.send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true,userGesture:true},session,timeoutMs);if(value.exceptionDetails)throw new Error(value.exceptionDetails.text);return value.result.value;};
 const inspect=async(timeoutMs=15000)=>checkedState(await evaluate(inspectExpression,timeoutMs));
 const settle=()=>evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
 let state;
 bootstrap.phase="gpui-initialization";bootstrap.started=Date.now();
 const bootstrapDeadline=bootstrap.started+bootstrap.budgetMs;
 while(Date.now()<bootstrapDeadline){
  state=await inspect(Math.max(1,bootstrapDeadline-Date.now()));
  if(selectedReady(state))break;
  await sleep(Math.min(100,Math.max(0,bootstrapDeadline-Date.now())));
 }
 bootstrap.elapsedMs=Date.now()-bootstrap.started;
 if(!selectedReady(state))throw new Error(`The actual GPUI viewer did not receive all selected terminal states within its ${bootstrap.budgetMs}ms startup budget: ${JSON.stringify(state)}`);
 bootstrap.phase="interactive";
 await writeFile(path.join(output,'browser-bootstrap-proof.json'),jsonArtifact(bootstrap));
 const platform=await evaluate('({platform:navigator.platform,userAgent:navigator.userAgent,secureContext:window.isSecureContext,devicePixelRatio})');
 const expected=process.env.TERMINAL_EXPECT_TEXT;if(expected&&!state.terminals.some(t=>t.text.includes(expected)))throw new Error(`The viewer did not render expected text ${expected}`);
 if(state.terminals.some(t=>t.paint_failures!==0))throw new Error('The GPUI terminal reported a paint failure.');
 if(process.env.TERMINAL_BROWSER_INPUT==='1'){
   const before=state.sessions.map(s=>s.id).sort((a,b)=>a-b);
   const checked=[];
   const requiredIds=Object.keys(expectedSessionOs).filter(key=>/^[0-9]+$/.test(key)).map(Number);
   if(requiredIds.some(id=>!state.selected.includes(id)))throw new Error('The configured OS proof includes a route that is not selected. Select every required route before running input acceptance.');
   for(const initial of visibleTerminals(state)){
     let terminal=initial;
     if(!terminal.live){if(requiredIds.includes(terminal.id))throw new Error(`Required terminal ${terminal.id} is unavailable; its input route was not verified.`);continue;}
     if(!terminal.controller){
       // GPUI reports the actual visible button's painted text bounds.
       const {x,y}=controlPoint(terminal);
       await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});
       await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});
       for(let i=0;i<100;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);if(terminal?.enabled)break;await sleep(50);}
       if(!terminal?.enabled)throw new Error(`Could not acquire terminal ${initial.id} through its visible Take control button.`);
     }
     const x=terminal.viewport.x+terminal.cell.width*2,y=terminal.viewport.y+terminal.cell.height*2;
     const focusProof={id:terminal.id,before:{dom:await evaluate('window.terminalFocusMetadata()'),gpui:state.input_focus,trace:state.focus_trace,inputStats:terminal.input_stats}};
     await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
     await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});await settle();
     state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);
     focusProof.afterClick={dom:await evaluate('window.terminalFocusMetadata()'),gpui:state.input_focus,trace:state.focus_trace,inputStats:terminal?.input_stats};
     await writeFile(path.join(output,`browser-focus-${initial.id}.json`),jsonArtifact(focusProof));
     if(checked.length===0){
       const modifiers=/Mac/.test(platform.platform)?4:10;
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'f',code:'KeyF',windowsVirtualKeyCode:70,modifiers});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'f',code:'KeyF',windowsVirtualKeyCode:70,modifiers});
       for(let i=0;i<100;i++){state=await inspect();if(state.terminals.some(t=>t.id===terminal.id&&t.find_open))break;await sleep(50);}
       focusProof.afterShortcut={dom:await evaluate('window.terminalFocusMetadata()'),gpui:state.input_focus,trace:state.focus_trace,terminals:visibleTerminals(state).map(t=>({id:t.id,findOpen:t.find_open,inputStats:t.input_stats}))};
       await writeFile(path.join(output,`browser-focus-${initial.id}.json`),jsonArtifact(focusProof));
       if(!state.terminals.some(t=>t.id===terminal.id&&t.find_open))throw new Error('The browser platform find shortcut did not reach the focused terminal.');
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
       for(let i=0;i<100;i++){state=await inspect();if(state.terminals.some(t=>t.id===terminal.id&&!t.find_open))break;await sleep(50);}
       if(!state.terminals.some(t=>t.id===terminal.id&&!t.find_open))throw new Error('Escape did not close terminal find and restore terminal focus.');
     }
     // Tests use dedicated shell sessions. Clear any unfinished command from a
     // failed prior run before entering a new command through real key events.
     await call('Input.dispatchKeyEvent',{type:'keyDown',key:'u',code:'KeyU',windowsVirtualKeyCode:85,modifiers:2});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'u',code:'KeyU',windowsVirtualKeyCode:85,modifiers:2});
     const marker=`BROWSER_${nonce}_${terminal.id}`;
     const title=state.sessions.find(s=>s.id===terminal.id)?.title;
     const expectedOs=expectedSessionOs[String(terminal.id)] ?? expectedSessionOs[title];
     if(Object.keys(expectedSessionOs).length&&!expectedOs)throw new Error(`No expected operating system is configured for terminal ${terminal.id} (${title}). Use its stable numeric ID or current title.`);
     // Split the marker in the source command so echoed input cannot satisfy the output assertion.
     await call('Input.insertText',{text:`printf 'BROWSER_${nonce}_%s\\n' ${terminal.id}; uname -s`});
     await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
     for(let i=0;i<300;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);if(terminal&&['Darwin','Linux'].includes(commandOs(terminal,marker)))break;await sleep(50);}
     if(!terminal||!['Darwin','Linux'].includes(commandOs(terminal,marker)))throw new Error(`Real browser keyboard input did not execute marker and uname as consecutive output lines in terminal ${initial.id}.`);
     if(expectedOs&&commandOs(terminal,marker)!==expectedOs)throw new Error(`Terminal ${title} ran on ${commandOs(terminal,marker)}; expected ${expectedOs}.`);
     const proof={id:initial.id,title,marker,os:commandOs(terminal,marker),offset:terminal.offset};checked.push(proof);
     if(process.env.TERMINAL_BROWSER_TAB==='1'){
       const functionName=`tab_${nonce}_${terminal.id}_complete`,tabMarker=`TAB_${nonce}_${terminal.id}`;
       await call('Input.insertText',{text:`${functionName}() { printf 'TAB_${nonce}_%s\\n' ${terminal.id}; }`});
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
       await call('Input.insertText',{text:functionName.slice(0,-4)});
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Tab',code:'Tab',windowsVirtualKeyCode:9});
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
       for(let i=0;i<300;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);if(terminal&&outputLines(terminal).includes(tabMarker))break;await sleep(50);}
       if(!terminal||!outputLines(terminal).includes(tabMarker))throw new Error(`Real Tab command completion failed in terminal ${initial.id}.`);
       proof.reloadMarker=tabMarker;proof.offset=terminal.offset;
       await writeFile(path.join(output,`browser-tab-${initial.id}.json`),jsonArtifact({id:initial.id,functionName,tabMarker,completed:true}));
       await call('Input.insertText',{text:`unset -f ${functionName}`});await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
     }
     if(process.env.TERMINAL_BROWSER_STYLE==='1'){
       const colorMarker=`RGB_${nonce}_${terminal.id}`,linkLabel=`LINK_${nonce}_${terminal.id}`;
       const linkUrl=`${new URL(url).origin}/health?terminal-link-proof=${nonce}-${terminal.id}`;
       await call('Input.insertText',{text:`printf '\\033[38;2;235;76;88mRGB_${nonce}_%s\\033[0m\\n\\033]8;;${linkUrl}\\007LINK_${nonce}_%s\\033]8;;\\007\\n' ${terminal.id} ${terminal.id}`});
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
       for(let i=0;i<200;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);if(terminal&&outputLines(terminal).includes(colorMarker)&&outputLines(terminal).includes(linkLabel)&&terminal.foreground_colors.includes('#eb4c58'))break;await sleep(50);}
       if(!terminal||!outputLines(terminal).includes(colorMarker)||!outputLines(terminal).includes(linkLabel)||!terminal.foreground_colors.includes('#eb4c58'))throw new Error(`Truecolor and OSC8 output did not render in terminal ${initial.id}.`);
       const colorRows=terminal.text.split('\n'),colorRow=colorRows.findIndex(line=>line.trim()===colorMarker),colorColumn=colorRows[colorRow].indexOf(colorMarker);
       const styleShot=await call('Page.captureScreenshot',{format:'png'}),styleBytes=Buffer.from(styleShot.data,'base64');
       const pixels=pngMetrics(styleBytes,{x:terminal.viewport.x+colorColumn*terminal.cell.width,y:terminal.viewport.y+colorRow*terminal.cell.height,width:colorMarker.length*terminal.cell.width,height:terminal.cell.height});
       if(pixels.redProbePixels<3)throw new Error(`Terminal ${initial.id}'s compositor image did not paint the requested truecolor marker.`);
       await writeFile(path.join(output,`browser-style-${initial.id}.png`),styleBytes);
       const rows=terminal.text.split('\n'),row=rows.findIndex(line=>line.trim()===linkLabel),column=rows[row].indexOf(linkLabel);
       const x=terminal.viewport.x+(column+1.5)*terminal.cell.width,y=terminal.viewport.y+(row+.5)*terminal.cell.height,modifiers=/Mac/.test(platform.platform)?4:2;
       await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y,modifiers});await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1,modifiers});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1,modifiers});
       let opened;
       for(let i=0;i<100;i++){const targets=await cdp.send('Target.getTargets');opened=targets.targetInfos.find(target=>target.url===linkUrl);if(opened)break;await sleep(50);}
       if(!opened)throw new Error(`Modifier-clicking the actual OSC8 cell did not open its URL for terminal ${initial.id}.`);
       await cdp.send('Target.closeTarget',{targetId:opened.targetId});await call('Page.bringToFront');await settle();
       await writeFile(path.join(output,`browser-style-${initial.id}.json`),jsonArtifact({id:initial.id,colorMarker,linkLabel,linkUrl,color:'#eb4c58',pixels,opened:true}));
       proof.reloadMarker=linkLabel;proof.offset=terminal.offset;
     }
     if(process.env.TERMINAL_BROWSER_CLIPBOARD==='1'){
       const clipMarker=`CLIP_${nonce}_${terminal.id}`,first=`${clipMarker}_α界é_é`,last=`END_${nonce}_${terminal.id}`,expected=`${first}\n${last}`,ready=`CLIPREADY_${nonce}_${terminal.id}`,receipt=`CLIPBYTES_${nonce}_${terminal.id}`;
       // This raw PTY fixture proves the copied frontend value returns over SSH
       // as the exact bracketed UTF-8 bytes, independently of shell echo/quoting.
       const script=`import os,select,termios,tty\nold=termios.tcgetattr(0)\ntty.setraw(0)\ntry:\n text=${JSON.stringify(expected)}\n os.write(1,('\\x1b[2J\\x1b[H\\x1b[?2004h\\x1b[38;2;235;76;88m'+text.replace('\\n','\\r\\n')+'\\x1b[0m\\r\\n${ready}\\r\\n').encode())\n data=b''\n while not data.endswith(b'\\x1b[201~') and len(data)<4096:\n  if not select.select([0],[],[],30)[0]: raise RuntimeError('fixture input timeout')\n  data+=os.read(0,4096)\n good=data==b'\\x1b[200~'+text.encode()+b'\\x1b[201~'\n os.write(1,('\\r\\n${receipt}_'+('PASS' if good else 'FAIL')+'\\r\\n').encode())\nfinally:\n termios.tcsetattr(0,termios.TCSANOW,old)\n`;
       const encoded=Buffer.from(script).toString('base64'),command=`python3 -u -c 'import base64;exec(base64.b64decode("${encoded}"))'`;
       await evaluate(`navigator.clipboard.writeText(${JSON.stringify(command)})`);
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'v',code:'KeyV',windowsVirtualKeyCode:86,modifiers:10});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'v',code:'KeyV',windowsVirtualKeyCode:86,modifiers:10});
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
       for(let i=0;i<300;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);if(terminal&&outputLines(terminal).includes(ready))break;await sleep(50);}
       if(!terminal||!outputLines(terminal).includes(ready))throw new Error(`Physical clipboard paste did not start the fixture in terminal ${initial.id}.`);
       const rows=terminal.text.split('\n'),firstRow=rows.findIndex(line=>line.trim()===first),lastRow=rows.findIndex(line=>line.trim()===last);
       if(firstRow<0||lastRow!==firstRow+1)throw new Error(`Unicode clipboard fixture lines were not consecutive in terminal ${initial.id}.`);
       const x=terminal.viewport.x+.5*terminal.cell.width,y=terminal.viewport.y+(firstRow+.5)*terminal.cell.height,endX=terminal.viewport.x+(last.length-.5)*terminal.cell.width,endY=terminal.viewport.y+(lastRow+.5)*terminal.cell.height;
       await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
       await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',buttons:1,clickCount:1});
       await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:endX,y:endY,button:'left',buttons:1});
       await call('Input.dispatchMouseEvent',{type:'mouseReleased',x:endX,y:endY,button:'left',buttons:0,clickCount:1});
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'c',code:'KeyC',windowsVirtualKeyCode:67,modifiers:10});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'c',code:'KeyC',windowsVirtualKeyCode:67,modifiers:10});
       let matches=false;for(let i=0;i<100;i++){matches=await evaluate(`navigator.clipboard.readText().then(value=>value===${JSON.stringify(expected)})`);if(matches)break;await sleep(50);}
       if(!matches)throw new Error(`Physical copy did not preserve exact Unicode/multiline plain text in terminal ${initial.id}. Clipboard contents were not recorded.`);
       // Do not replace the clipboard with expected: paste back the actual copy.
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'v',code:'KeyV',windowsVirtualKeyCode:86,modifiers:10});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'v',code:'KeyV',windowsVirtualKeyCode:86,modifiers:10});
       for(let i=0;i<300;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);if(terminal&&outputLines(terminal).includes(`${receipt}_PASS`))break;await sleep(50);}
       if(!terminal||!outputLines(terminal).includes(`${receipt}_PASS`))throw new Error(`Copied Unicode/multiline clipboard bytes did not round-trip exactly through terminal ${initial.id}'s PTY.`);
       await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});
       const cleared=`SELECTION_CLEARED_${nonce}`;await evaluate(`navigator.clipboard.writeText(${JSON.stringify(cleared)})`);
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'c',code:'KeyC',windowsVirtualKeyCode:67,modifiers:10});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'c',code:'KeyC',windowsVirtualKeyCode:67,modifiers:10});await settle();
       const selectionCleared=await evaluate(`navigator.clipboard.readText().then(value=>value===${JSON.stringify(cleared)})`);
       if(!selectionCleared)throw new Error('A click did not clear terminal selection before memory acceptance. Clipboard contents were not recorded.');
       proof.reloadMarker=`${receipt}_PASS`;proof.offset=terminal.offset;
       await writeFile(path.join(output,`browser-clipboard-${initial.id}.json`),jsonArtifact({id:initial.id,clipboard:'isolated Linux headless fixture',pasteExecuted:true,copyMatched:true,unicode:true,multiline:true,ansiStripped:true,ptyBytesMatched:true,selectionCleared}));
       const clipShot=await call('Page.captureScreenshot',{format:'png'}),clipBytes=Buffer.from(clipShot.data,'base64');requirePaint(pngMetrics(clipBytes));
       await writeFile(path.join(output,`browser-clipboard-${initial.id}.png`),clipBytes);
     }
     if(process.env.TERMINAL_BROWSER_RESIZE==='1'){
       const uuid=state.sessions.find(s=>s.id===initial.id)?.uuid;
       if(!uuid)throw new Error(`Vertical resize needs a stable UUID for terminal ${initial.id}.`);
       const phases=[],bottomPrefix=`V_BOTTOM_${nonce}_${initial.id}`;
       let shellPid;
       for(const [step,height] of [800,500,1000,800].entries()){
         await call('Emulation.setDeviceMetricsOverride',{width:1280,height,deviceScaleFactor:1,mobile:false});
         for(let i=0;i<300;i++){
           state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);
           const fit=terminal&&terminal.rows===Math.floor(terminal.viewport.height/terminal.cell.height);
           if(terminal?.enabled&&fit&&(step===0||terminal.rows!==phases.at(-1).rows))break;
           await sleep(50);
         }
         if(!terminal?.enabled||terminal.rows!==Math.floor(terminal.viewport.height/terminal.cell.height)||(step&&terminal.rows===phases.at(-1).rows))throw new Error(`Vertical resize did not fit terminal ${initial.id} to height ${height}.`);
         const expected={cols:terminal.cols,rows:terminal.rows};
         const x=terminal.viewport.x+terminal.cell.width*2,y=terminal.viewport.y+terminal.cell.height*2;
         await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
         await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});await settle();
         const status=`V_SIZE_${nonce}_${initial.id}_${step}`,bottom=`${bottomPrefix}_${step}`;
         // Query the actual PTY, then address its last row. Split output labels in
         // the source command so an echoed command cannot satisfy the assertions.
         const command=`${step===0?`export VERTICAL_KEEP=${nonce}; `:''}set -- $(stty size); printf '\\033[2J\\033[HV_SIZE_${nonce}_${initial.id}_%s=%s,%s,%s,%s\\n' ${step} "$1" "$2" "$$" "$VERTICAL_KEEP"; printf '\\033[%s;1H\\033[38;2;235;76;88mV_BOTTOM_${nonce}_${initial.id}_%s\\033[0m' "$1" ${step}`;
         await call('Input.insertText',{text:command});
         await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
         let fields;
         for(let i=0;i<300;i++){
           state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);
           const lines=terminal?.text.split('\n')||[];
           const output=lines.find(line=>line.startsWith(status+'='));
           fields=output?.slice(status.length+1).trim().split(',');
           if(fields?.length===4&&lines.at(-1)?.startsWith(bottom))break;
           await sleep(50);
         }
         if(!fields||Number(fields[0])!==expected.rows||Number(fields[1])!==expected.cols||fields[3]!==nonce||!terminal?.text.split('\n').at(-1)?.startsWith(bottom)||terminal.cursor.y!==expected.rows-1)throw new Error(`Vertical resize failed PTY dimensions, environment, cursor or bottom-row output in terminal ${initial.id} at height ${height}.`);
         if(shellPid&&fields[2]!==shellPid)throw new Error(`Vertical resize restarted terminal ${initial.id}'s shell.`);
         shellPid=fields[2];
         if(state.sessions.find(s=>s.id===initial.id)?.uuid!==uuid)throw new Error(`Vertical resize changed terminal ${initial.id}'s UUID.`);
         phases.push({height,...expected,pid:shellPid,cursor:terminal.cursor,offset:terminal.offset,bottom});
         const image=Buffer.from((await call('Page.captureScreenshot',{format:'png'})).data,'base64');requirePaint(pngMetrics(image));
         const bottomPaint=pngMetrics(image,{x:terminal.viewport.x,y:terminal.viewport.y+(expected.rows-1)*terminal.cell.height,width:Math.min(terminal.viewport.width,expected.cols*terminal.cell.width),height:terminal.cell.height});
         if(bottomPaint.redProbePixels<20)throw new Error(`The actual bottom-row RGB sentinel was not painted in terminal ${initial.id} at height ${height}.`);
         phases.at(-1).bottomRedPixels=bottomPaint.redProbePixels;
         await writeFile(path.join(output,`browser-resize-${initial.id}-${height}-${step}.png`),image);
       }
       if(!(phases[1].rows<phases[0].rows&&phases[2].rows>phases[0].rows&&phases[3].rows===phases[0].rows&&phases.every(p=>p.cols===phases[0].cols)))throw new Error(`Vertical resize did not shrink, grow and restore terminal ${initial.id} at constant width.`);
       const restored=`V_RESTORED_${nonce}_${initial.id}`;
       await call('Input.insertText',{text:`printf '\\r\\nV_RESTORED_${nonce}_%s\\n' ${initial.id}`});
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
       for(let i=0;i<300;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===initial.id);if(terminal&&outputLines(terminal).includes(restored))break;await sleep(50);}
       if(!terminal||!outputLines(terminal).includes(restored))throw new Error(`The resized terminal ${initial.id} stopped accepting shell input.`);
       proof.reloadMarker=restored;proof.uuid=uuid;proof.shellPid=shellPid;
       proof.offset=terminal.offset;
       await writeFile(path.join(output,`browser-resize-${initial.id}.json`),jsonArtifact({id:initial.id,uuid,phases,restored:true}));
     }

   }
   if(!checked.length)throw new Error('No live visible terminal was available for browser input.');
   if(requiredIds.some(id=>!checked.some(proof=>proof.id===id)))throw new Error('A required terminal input route was not checked.');
   if(process.env.TERMINAL_BROWSER_RELOAD!=='0'){
   const beforeReloadOrigin=await evaluate('performance.timeOrigin');let afterReloadOrigin,restoredAfterReload=false;
   reloadBootstrap={phase:'gpui-reload',started:Date.now(),budgetMs:60000};
   const reloadDeadline=reloadBootstrap.started+reloadBootstrap.budgetMs;
   await call('Page.reload');
   while(Date.now()<reloadDeadline){
     try{
       afterReloadOrigin=await evaluate('performance.timeOrigin',Math.max(1,reloadDeadline-Date.now()));
       if(afterReloadOrigin!==beforeReloadOrigin){state=await inspect(Math.max(1,reloadDeadline-Date.now()));if(selectedReady(state)&&checked.every(proof=>state.terminals?.some(t=>t.id===proof.id&&t.offset>=proof.offset&&outputLines(t).includes(proof.reloadMarker||proof.marker)))){restoredAfterReload=true;break;}}
     }catch(error){if(!/Execution context was destroyed|Cannot find context|Cannot find default execution context/.test(error.message))throw error;}
     await sleep(Math.min(100,Math.max(0,reloadDeadline-Date.now())));
   }
   reloadBootstrap.elapsedMs=Date.now()-reloadBootstrap.started;
   reloadBootstrap.phase=restoredAfterReload?'interactive':'reload-failed';
   await writeFile(path.join(output,'browser-reload-bootstrap-proof.json'),jsonArtifact(reloadBootstrap));
   if(!restoredAfterReload||afterReloadOrigin===beforeReloadOrigin||!state.connected||JSON.stringify(before)!==JSON.stringify(state.sessions.map(s=>s.id).sort((a,b)=>a-b))||!checked.every(proof=>state.terminals.some(t=>t.id===proof.id&&outputLines(t).includes(proof.reloadMarker||proof.marker))))throw new Error('A new browser document did not restore the same live terminals and their output.');
   for(const proof of checked.filter(p=>p.uuid)){
     if(state.sessions.find(s=>s.id===proof.id)?.uuid!==proof.uuid)throw new Error(`Browser reload changed resized terminal ${proof.id}'s UUID.`);
     let terminal=state.terminals.find(t=>t.id===proof.id);
     const x=terminal.viewport.x+terminal.cell.width*2,y=terminal.viewport.y+terminal.cell.height*2;
     await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});await settle();
     const continuation=`V_AFTER_${nonce}_${proof.id}=${proof.shellPid},${nonce}`;
     await call('Input.insertText',{text:`printf 'V_AFTER_${nonce}_%s=%s,%s\\n' ${proof.id} "$$" "$VERTICAL_KEEP"`});
     await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
     for(let i=0;i<300;i++){state=await inspect();terminal=state.terminals.find(t=>t.id===proof.id);if(terminal&&outputLines(terminal).includes(continuation))break;await sleep(50);}
     if(!terminal||!outputLines(terminal).includes(continuation))throw new Error(`Reload after vertical resize did not preserve terminal ${proof.id}'s process and environment.`);
     proof.afterReloadPidVerified=true;
   }
   if(checked.some(p=>p.uuid))await writeFile(path.join(output,'browser-resize-reloaded.png'),Buffer.from((await call('Page.captureScreenshot',{format:'png'})).data,'base64'));
   await writeFile(path.join(output,'browser-input-reload-proof.json'),jsonArtifact({checked,pageOrigins:{before:beforeReloadOrigin,after:afterReloadOrigin},sessionsAfterReload:state.sessions}));
   }else{state=await inspect();await writeFile(path.join(output,'browser-input-proof.json'),jsonArtifact({checked,reloadChecked:false,sessions:state.sessions}));}
 }


 if(process.env.TERMINAL_BROWSER_MEMORY==='1'){
   state=await inspect();
   const id=state.selected[0],scratch=state.sessions.find(session=>!state.selected.includes(session.id));
   if(!scratch)throw new Error('Memory acceptance requires a dedicated host with at least three sessions.');
   if(!state.memory_saver?.enabled)throw new Error('Memory acceptance requires the default enabled memory saver.');
   const clickTab=async target=>{
     state=await inspect();const b=state.tabs?.find(tab=>tab.group===0&&tab.id===target);
     if(!b||b.width<=0)throw new Error(`No actual tab geometry exists for session ${target}.`);
     const x=b.x+b.width/2,y=b.y+b.height/2;
     await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});await settle();
   };
   const sendCommand=async text=>{
     state=await inspect();const terminal=state.terminals.find(terminal=>terminal.id===id);
     if(!terminal?.enabled)throw new Error('Memory acceptance cannot type before the host grants control and restores a fresh snapshot.');
     const x=terminal.viewport.x+20,y=terminal.viewport.y+20;
     await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});await settle();
     await call('Input.dispatchKeyEvent',{type:'keyDown',key:'u',code:'KeyU',windowsVirtualKeyCode:85,modifiers:2});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'u',code:'KeyU',windowsVirtualKeyCode:85,modifiers:2});
     await call('Input.insertText',{text});await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
   };
   let target=state.terminals.find(terminal=>terminal.id===id);
   if(!target?.controller){const {x,y}=controlPoint(target);await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});for(let i=0;i<200;i++){state=await inspect();target=state.terminals.find(t=>t.id===id);if(target?.enabled)break;await sleep(50);}}
   const saved=`MEMORY_${nonce}_SAVED`,resumed=`MEMORY_${nonce}_RESUMED`;
   await writeFile(path.join(output,'browser-memory-start.json'),jsonArtifact({timeOrigin:await evaluate('performance.timeOrigin'),state}));
   await sendCommand(`i=0; while [ "$i" -lt 12000 ]; do printf '%05d dense browser memory history retained across tab parking\\n' "$i"; i=$((i+1)); done; printf 'MEMORY_${nonce}_%s\\n' SAVED`);
   for(let i=0;i<600;i++){state=await inspect();target=state.terminals.find(t=>t.id===id);if(!state.connected){await writeFile(path.join(output,'browser-memory-disconnect.json'),jsonArtifact({state,firstProtocolFailure:cdp.firstFailure,frames:cdp.events.filter(event=>event.params?.summary).slice(-20)}));throw new Error(`Memory probe lost its WebSocket connection: ${state.notice}`);}if(target&&target.history_rows>=9500&&outputLines(target).includes(saved))break;await sleep(100);}
   if(!target||target.history_rows<9500||!outputLines(target).includes(saved))throw new Error('The real memory probe did not create the configured approximately10k-line history.');
   const before={offset:target.offset,historyRows:target.history_rows};
   await clickTab(scratch.id);
   let loaded;
   for(let i=0;i<300;i++){state=await inspect();if(state.selected[0]===scratch.id&&state.terminals.some(t=>t.id===scratch.id)){loaded=state.memory_saver.resident_count;break;}await sleep(100);}
   if(!loaded)throw new Error('Selecting the third tab did not load its real terminal.');
   for(let i=0;i<750;i++){state=await inspect();if(state.memory_saver.parked.some(p=>p.id===id&&p.detached)&&!state.terminals.some(t=>t.id===id))break;await sleep(100);}
   const parked=state.memory_saver;
   if(!parked.parked.some(p=>p.id===id&&p.detached)||parked.resident_count>=loaded||state.terminals.some(t=>t.id===id))throw new Error('The inactive dense replica did not detach and release its resident entity after the idle timeout.');
   const parkedShot=await call('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,'browser-memory-parked.png'),Buffer.from(parkedShot.data,'base64'));requirePaint(pngMetrics(Buffer.from(parkedShot.data,'base64')));
   await clickTab(id);
   for(let i=0;i<600;i++){state=await inspect();target=state.terminals.find(t=>t.id===id);if(target?.enabled&&target.offset>=before.offset&&target.history_rows>=9500&&outputLines(target).includes(saved))break;await sleep(100);}
   if(!target?.enabled||target.offset<before.offset||target.history_rows<9500||!outputLines(target).includes(saved))throw new Error('Waking the parked tab did not restore its fresh checkpoint and dense history.');
   await sendCommand(`printf 'MEMORY_${nonce}_%s\\n' RESUMED`);
   for(let i=0;i<300;i++){state=await inspect();target=state.terminals.find(t=>t.id===id);if(target&&outputLines(target).includes(resumed))break;await sleep(100);}
   if(!target||!outputLines(target).includes(resumed))throw new Error('Real keyboard input did not resume after parked-tab restoration.');
   await writeFile(path.join(output,'browser-memory-proof.json'),jsonArtifact({id,scratch:scratch.id,before,residentBeforeParking:loaded,parked,after:{offset:target.offset,historyRows:target.history_rows,memory:state.memory_saver},saved,resumed}));
 }

 if(process.env.TERMINAL_BROWSER_MIRROR==='1'){
   let first=visibleTerminals(state).find(t=>t.live);
   if(!first)throw new Error('Mirror acceptance requires a live terminal.');
   if(!first.controller){
     const {x,y}=controlPoint(first);
     await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});
     for(let i=0;i<200;i++){state=await inspect();first=state.terminals.find(t=>t.id===first.id);if(first?.enabled)break;await sleep(50);}
     if(!first?.enabled)throw new Error('The first browser could not acquire control before mirror acceptance.');
   }
   const target2=await cdp.send('Target.createTarget',{url:'about:blank',newWindow:true});
   const attached2=await cdp.send('Target.attachToTarget',{targetId:target2.targetId,flatten:true});
   const mirrorCall=(method,params={})=>cdp.send(method,params,attached2.sessionId);
   activeMirrorCall=mirrorCall;
   await mirrorCall('Page.enable');await mirrorCall('Runtime.enable');
   await mirrorCall('Emulation.setDeviceMetricsOverride',{width:960,height:700,deviceScaleFactor:1,mobile:false});
   await mirrorCall('Page.navigate',{url});
   const mirrorInspect=async()=>{const result=await mirrorCall('Runtime.evaluate',{expression:inspectExpression,returnByValue:true});return checkedState(result.result.value);};
   let mirror;
   for(let i=0;i<600;i++){mirror=await mirrorInspect();if(mirror.connected&&visibleTerminals(mirror).length===visibleTerminals(state).length&&visibleTerminals(mirror).every(t=>t.viewport?.width>0&&t.text.trim()))break;await sleep(100);}
   const terminal=visibleTerminals(mirror).find(t=>t.live);
   if(!terminal||terminal.controller)throw new Error('The second browser view must start as a read-only mirror of the existing controller.');
   const initial=state.terminals.find(t=>t.id===terminal.id);
   if(!initial||terminal.offset<initial.offset)throw new Error('The second browser view did not restore the existing terminal output.');
   const {x,y}=controlPoint(terminal);
   await mirrorCall('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await mirrorCall('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});
   for(let i=0;i<200;i++){mirror=await mirrorInspect();state=await inspect();if(mirror.terminals.some(t=>t.id===terminal.id&&t.enabled)&&state.terminals.some(t=>t.id===terminal.id&&!t.controller))break;await sleep(50);}
   if(!mirror.terminals.some(t=>t.id===terminal.id&&t.enabled)||!state.terminals.some(t=>t.id===terminal.id&&!t.controller))throw new Error('Explicit control transfer did not update both browser mirrors.');
   await mirrorCall('Input.dispatchMouseEvent',{type:'mousePressed',x:terminal.viewport.x+20,y:terminal.viewport.y+20,button:'left',clickCount:1});await mirrorCall('Input.dispatchMouseEvent',{type:'mouseReleased',x:terminal.viewport.x+20,y:terminal.viewport.y+20,button:'left',clickCount:1});
   const mirrorMarker=`MIRROR_${nonce}_TRANSFERRED`;
   await mirrorCall('Input.insertText',{text:`printf 'MIRROR_${nonce}_%s\\n' TRANSFERRED`});
   await mirrorCall('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await mirrorCall('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
   for(let i=0;i<300;i++){mirror=await mirrorInspect();state=await inspect();if([mirror,state].every(s=>s.terminals.some(t=>t.id===terminal.id&&outputLines(t).includes(mirrorMarker))))break;await sleep(50);}
   const mirrorShot=await mirrorCall('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,'browser-mirror.png'),Buffer.from(mirrorShot.data,'base64'));
   requirePaint(pngMetrics(Buffer.from(mirrorShot.data,'base64')));
   await writeFile(path.join(output,'browser-mirror-proof.json'),jsonArtifact({marker:mirrorMarker,first:state,second:mirror}));
   if(![mirror,state].every(s=>s.terminals.some(t=>t.id===terminal.id&&outputLines(t).includes(mirrorMarker))))throw new Error('Input from the new controller did not appear in both browser mirrors.');
   if(process.env.TERMINAL_BROWSER_PAN==='1'){
     let owner=visibleTerminals(state).find(t=>t.controller&&t.live&&mirror.terminals.some(other=>other.id===t.id&&!other.controller&&other.mirror_clip));
     if(!owner)throw new Error('No asymmetric read-only terminal is available for panning.');
     const id=owner.id,cols=owner.cols,rows=owner.rows;
     const left=`LEFT_${nonce}`,right=`RIGHT_${nonce}`,bottom=`BOTTOM_${nonce}`;
     const enter=`printf '\\033[?1049h\\033[2J\\033[H${left}\\033[1;${cols-right.length+1}H${right}\\033[${rows};${cols-bottom.length+1}H${bottom}\\033[4;1H'`;
     const sendOwner=async text=>{
       state=await inspect();owner=state.terminals.find(t=>t.id===id);
       await call('Page.bringToFront');
       const x=owner.viewport.x+20,y=owner.viewport.y+20;
       await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});await settle();
       await call('Input.dispatchKeyEvent',{type:'keyDown',key:'u',code:'KeyU',windowsVirtualKeyCode:85,modifiers:2});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'u',code:'KeyU',windowsVirtualKeyCode:85,modifiers:2});
       await call('Input.insertText',{text});await call('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});await call('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
     };
     try{
       await sendOwner(enter);
       let view;
       for(let i=0;i<200;i++){mirror=await mirrorInspect();view=mirror.terminals.find(t=>t.id===id);if(view?.alternate_screen&&view.text.includes(left)&&view.text.includes(right)&&view.text.includes(bottom))break;await sleep(50);}
       if(!view?.alternate_screen||!view.text.includes(bottom))throw new Error('The shared alternate-screen TUI did not reach the read-only mirror.');
       const before={viewport:view.viewport,clip:view.mirror_clip,cols:view.cols,rows:view.rows};
       if(!(before.viewport.width>before.clip.width&&before.viewport.height>before.clip.height))throw new Error('The pan probe did not produce an asymmetric viewport in both axes.');
       const beforeShot=await mirrorCall('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,'browser-pan-before.png'),Buffer.from(beforeShot.data,'base64'));
       const x=before.clip.x+before.clip.width/2,y=before.clip.y+Math.min(70,before.clip.height/2);
       await mirrorCall('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:10000,deltaY:0});
       await mirrorCall('Input.dispatchMouseEvent',{type:'mouseWheel',x,y,deltaX:0,deltaY:10000});
       for(let i=0;i<200;i++){mirror=await mirrorInspect();view=mirror.terminals.find(t=>t.id===id);const b=view.viewport,c=view.mirror_clip;if(b.x+b.width<=c.x+c.width+1&&b.y+b.height<=c.y+c.height+1)break;await sleep(50);}
       const b=view.viewport,c=view.mirror_clip;
       if(!(b.x+b.width<=c.x+c.width+1&&b.y+b.height<=c.y+c.height+1))throw new Error('Read-only panning could not reveal the last shared row and column.');
       state=await inspect();owner=state.terminals.find(t=>t.id===id);if(owner.cols!==cols||owner.rows!==rows)throw new Error('Read-only panning resized the shared PTY.');
       await sleep(2300);const afterShot=await mirrorCall('Page.captureScreenshot',{format:'png'});requirePaint(pngMetrics(Buffer.from(afterShot.data,'base64')));await writeFile(path.join(output,'browser-pan-after.png'),Buffer.from(afterShot.data,'base64'));
       await writeFile(path.join(output,'browser-pan-proof.json'),jsonArtifact({id,before,after:{viewport:b,clip:c,cols:view.cols,rows:view.rows},owner:{cols:owner.cols,rows:owner.rows},labels:{left,right,bottom}}));
     }finally{await sendOwner("printf '\\033[?1049l'");}
   }

 }

 await sleep(200);const shot=await call('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,'browser-terminal.png'),Buffer.from(shot.data,'base64'));
 const pixels=pngMetrics(Buffer.from(shot.data,'base64'));requirePaint(pixels);
 await writeFile(path.join(output,'browser-proof.json'),jsonArtifact({browser:platform,pixels,state,version:await cdp.send('Browser.getVersion')}));
 await writeFile(path.join(output,'browser-network-events.json'),jsonArtifact({first:cdp.firstNetwork,recent:cdp.network,firstFailure:cdp.firstFailure}));
 await writeFile(path.join(output,'browser-runtime-events.json'),jsonArtifact(cdp.events.filter(event=>['Runtime.consoleAPICalled','Runtime.exceptionThrown','Log.entryAdded','Page.frameNavigated'].includes(event.method)||event.method.startsWith('Network.webSocket'))));
 console.log(redact(JSON.stringify({proof:'same-gpui-terminal-remote-browser',clientPlatform:platform.platform,hostOs:state.host?.os,hostArch:state.host?.arch,sessions:state.terminals.map(t=>({id:t.id,cols:t.cols,rows:t.rows,colors:t.foreground_colors?.length,paintFailures:t.paint_failures})),inputChecked:process.env.TERMINAL_BROWSER_INPUT==='1',mirrorChecked:process.env.TERMINAL_BROWSER_MIRROR==='1',output})));
 await cdp.send('Browser.close');
}catch(error){
 if(reloadBootstrap)await writeFile(path.join(output,'browser-reload-bootstrap-proof.json'),jsonArtifact({...reloadBootstrap,elapsedMs:reloadBootstrap.elapsedMs??Date.now()-reloadBootstrap.started}));
 await writeFile(path.join(output,'browser-bootstrap-proof.json'),jsonArtifact({...bootstrap,elapsedMs:bootstrap.elapsedMs??Date.now()-bootstrap.started}));
 if(cdp?.socket.readyState===WebSocket.OPEN){
   for(const [name,call] of [['browser-failure',activeCall],['browser-mirror-failure',activeMirrorCall]]){
     if(!call)continue;
     try{const shot=await call('Page.captureScreenshot',{format:'png'});await writeFile(path.join(output,`${name}.png`),Buffer.from(shot.data,'base64'));}catch{}
     try{const focus=await call('Runtime.evaluate',{expression:'window.terminalFocusMetadata?.()',returnByValue:true});await writeFile(path.join(output,`${name}-dom-focus.json`),jsonArtifact(focus.result.value));}catch{}
     try{const state=await call('Runtime.evaluate',{expression:inspectExpression,returnByValue:true});await writeFile(path.join(output,`${name}.json`),jsonArtifact(state.result.value));}catch{}
   }
 }
 const events=cdp?.events.filter(event=>['Runtime.consoleAPICalled','Runtime.exceptionThrown','Log.entryAdded','Page.frameNavigated'].includes(event.method)||event.method.startsWith('Network.webSocket')) || [];
 await writeFile(path.join(output,'browser-runtime-events.json'),jsonArtifact(events));
 await writeFile(path.join(output,'browser-first-protocol-failure.json'),jsonArtifact(cdp?.firstFailure||null));
 await writeFile(path.join(output,'browser-network-events.json'),jsonArtifact({first:cdp?.firstNetwork,recent:cdp?.network,firstFailure:cdp?.firstFailure}));
 const consoleText=events.filter(event=>event.method==='Runtime.consoleAPICalled').flatMap(event=>event.params.args.map(arg=>arg.value??arg.description??arg.type)).join('\n');
 await writeFile(path.join(output,'browser-error.txt'),redact(`${error.stack}\n${consoleText}\n${stderr}`));
 if(error instanceof Error){error.message=redact(error.message);error.stack=redact(error.stack);}
 throw error;
}
finally{await cleanup();}
