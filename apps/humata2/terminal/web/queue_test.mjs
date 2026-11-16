import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const rust=await readFile(process.argv[2],'utf8');
const source=rust.match(/inline_js\s*=\s*r#"(.*?)"#/s)?.[1];
if(!source)throw new Error('Could not load the production inline transport module.');
const transport=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
const {IncomingQueue}=transport;
const burst=new IncomingQueue();
for(let i=0;i<12000;i++)assert.equal(burst.push({kind:'text',text:JSON.stringify({type:'output',id:1,offset:i,bytes:'eA=='})}),true);
let count=0;
for(;;){const batch=burst.drain(256);if(!batch.length)break;for(const event of batch)assert.equal(JSON.parse(event.text).offset,count++);}
assert.equal(count,12000);
const bounded=new IncomingQueue(65536),event={kind:'text',text:'x'.repeat(4000)};
let accepted=0;while(bounded.push(event))accepted++;
assert.ok(accepted>1&&accepted<32);
assert.equal(bounded.drain(1).length,1);
assert.equal(bounded.push(event),true);
assert.equal(bounded.push(event),false,'partial drain must retain the remaining items in the memory budget');
assert.equal(bounded.drain(100).length,accepted);
assert.equal(bounded.push(event),true);
bounded.clear();assert.equal(bounded.drain().length,0);
assert.equal(bounded.push({kind:'opened'}),true);
assert.equal(bounded.push({kind:'text',text:'one'}),true);
assert.equal(bounded.push({kind:'closed',reason:'test'}),true);
assert.deepEqual(bounded.drain().map(event=>event.kind),['opened','text','closed']);
console.log('Incoming transport queue: tiny burst, FIFO, bounded overflow and partial drain passed.');

const sockets=[];
class Socket {
 static OPEN=1;
 constructor(url){this.url=url;this.readyState=1;this.bufferedAmount=0;this.closed=false;sockets.push(this);}
 close(){this.closed=true;this.readyState=3;}
 send(text){this.sent=text;}
}
globalThis.WebSocket=Socket;
const old=transport.terminal_socket_open('ws://fixture/old'),other=transport.terminal_socket_open('ws://fixture/other');
const late=sockets[0].onmessage;sockets[0].onmessage({data:'old queued output'});
transport.terminal_socket_close_all();transport.terminal_socket_close_all();
assert.ok(sockets.every(socket=>socket.closed));
assert.deepEqual(JSON.parse(transport.terminal_socket_drain(old)),[]);
assert.deepEqual(JSON.parse(transport.terminal_socket_drain(other)),[]);
assert.equal(transport.terminal_socket_send(old,'must not replay'),1);
const fresh=transport.terminal_socket_open('ws://fixture/fresh');
late({data:'late old output'});sockets[2].onmessage({data:'fresh output'});
assert.deepEqual(JSON.parse(transport.terminal_socket_drain(fresh)),[{kind:'text',text:'fresh output'}]);
transport.terminal_socket_close_all();
console.log('Socket lifecycle: idempotent close-all, discarded queued output and no stale delivery passed.');
