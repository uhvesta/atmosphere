// Scrub the private URL/credential from diagnostics before crossing into test outputs.
import {readFile,writeFile,readdir} from 'node:fs/promises';
import path from 'node:path';
const [urlFile,directory]=process.argv.slice(2);
const raw=(await readFile(urlFile,'utf8')).trim();
const url=new URL(raw);
const token=new URLSearchParams(url.hash.slice(1)).get('token');
const secrets=[raw,token].filter(Boolean);
for(const name of await readdir(directory)){
  if(!/\.(json|txt|log)$/.test(name))continue;
  const file=path.join(directory,name);
  let text=await readFile(file,'utf8');
  for(const secret of secrets)text=text.replaceAll(secret,'[redacted]');
  await writeFile(file,text,{mode:0o600});
}
