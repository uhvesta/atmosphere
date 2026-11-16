// Explicit input to a dedicated demo terminal; never automatically retries input.
import {readFile,stat} from 'node:fs/promises';
import path from 'node:path';

class DeliveryError extends Error {
  constructor(operation,message,cause){super(`${operation}: ${message}`,{cause});this.name='DeliveryError';this.operation=operation;}
}
const options={};
for(let i=2;i<process.argv.length;i++){
  const flag=process.argv[i];
  if(flag==='--take-control'){options.takeControl=true;continue;}
  if(!['--state','--id','--uuid','--file','--expect'].includes(flag)||!process.argv[i+1])throw new DeliveryError('Read options','Use --state DIR (--uuid UUID | --id NUMBER) --file INPUT [--expect REGEX] [--take-control].');
  options[flag.slice(2)]=process.argv[++i];
}
let id=options.id===undefined?undefined:Number(options.id);
const uuid=options.uuid?.toLowerCase();
if(!options.state||!options.file||((id!==undefined)===(uuid!==undefined))
  ||(id!==undefined&&(!Number.isSafeInteger(id)||id<1))
  ||(uuid!==undefined&&!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(uuid)))throw new DeliveryError('Read options','Specify a private demo state, exactly one existing terminal UUID or numeric route, and an input file.');
const info=await stat(options.file);
if(!info.isFile()||info.size===0||info.size>256*1024)throw new DeliveryError('Read input','Choose a regular nonempty input file of at most256KiB.');
const bytes=await readFile(options.file);
const token=(await readFile(path.join(options.state,'browser.token'),'utf8')).trim();
const {port}=JSON.parse(await readFile(path.join(options.state,'server.json'),'utf8'));
if(!Number.isInteger(port)||port<1024||port>65535)throw new DeliveryError('Locate host','The private server record has an invalid port; restart the dedicated demo.');
const pattern=options.expect?new RegExp(options.expect):null;
if(typeof WebSocket!=='function')throw new DeliveryError('Start connection','Use Node22+ through TERMINAL_LAB_NODE.');
const socket=new WebSocket(`ws://127.0.0.1:${port}/ws`);
const decoder=new TextDecoder();
let live=false,controller=false,sent=false,written=false,output='';
const send=frame=>socket.send(JSON.stringify(frame));
await new Promise((resolve,reject)=>{
  const timeout=setTimeout(()=>finish(new DeliveryError('Confirm input','Timed out. Input may already have reached the terminal; inspect it before retrying.')),15000);
  function finish(error){clearTimeout(timeout);socket.close();error?reject(error):resolve();}
  function maybeSend(){if(live&&controller&&!sent){sent=true;send({type:'input',id,seq:1,bytes:bytes.toString('base64')});}}
  socket.addEventListener('open',()=>send({type:'authenticate',protocol:3,token}));
  socket.addEventListener('error',()=>finish(new DeliveryError('Connect to host','Check the dedicated demo host. Do not resend uncertain input automatically.')));
  socket.addEventListener('message',event=>{
    try{
      const frame=JSON.parse(event.data);
      if(frame.type==='sessions'&&!sent){const terminal=frame.sessions.find(s=>uuid?s.uuid===uuid:s.id===id);if(!terminal)throw new DeliveryError('Open terminal',`Terminal ${uuid||id} does not exist; choose an existing dedicated demo terminal.`);id=terminal.id;send({type:'open',id});}
      if(frame.id!==undefined&&frame.id!==null&&frame.id!==id)return;
      if(frame.type==='snapshot'){live=frame.live&&frame.exited==null;send({type:'ack',id,offset:frame.offset});if(!live)throw new DeliveryError('Open terminal','This terminal is not writable; inspect its recovery notice before sending input.');maybeSend();}
      if(frame.type==='control'){controller=frame.controller;if(!controller){if(options.takeControl)send({type:'take_control',id});else throw new DeliveryError('Control terminal','Another viewer controls it. Use --take-control only when you intend to transfer that lease.');}maybeSend();}
      if(frame.type==='output'){const chunk=typeof frame.bytes==='string'?Buffer.from(frame.bytes,'base64'):new Uint8Array(frame.bytes);output=(output+decoder.decode(chunk,{stream:true})).slice(-128*1024);send({type:'ack',id,offset:frame.offset+chunk.length});}
      if(frame.type==='input_written'&&frame.seq===1)written=true;
      if(frame.type==='error')throw new DeliveryError('Use terminal',frame.message);
      if(written&&(!pattern||pattern.test(output)))finish();
    }catch(error){finish(error);}
  });
});
console.log(`Input written to terminal ${uuid||id}${pattern?' and expected output observed':''}. No automatic retry was performed.`);
