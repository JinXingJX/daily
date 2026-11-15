// Run the local Postman task-runtime simulation first (see docs/task-runtime.md).
// Exercises the built Electron app and real Writer handlers against a delayed model.
import { spawn } from 'node:child_process';
import { cp, mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createServer } from 'node:net';
import assert from 'node:assert/strict';
const repo=resolve(import.meta.dirname,'..');
const root=await mkdtemp(join(tmpdir(),'daily-task-runtime-'));
const modelUrl=process.env.DAILY_TASK_QA_MODEL_URL ?? 'http://127.0.0.1:4968/model';
const probe=createServer();
await new Promise((resolve,reject)=>{probe.once('error',reject);probe.listen(0,'127.0.0.1',resolve);});
const debugPort=probe.address().port;
await new Promise(resolve=>probe.close(resolve));
const require=createRequire(repo+'/package.json');
const {build}=require('esbuild');
await mkdir(root+'/workspace',{recursive:true});
await cp(resolve(repo,'../daily-writer-plugin/.daily-plugin/dist'),root+'/writer-build',{recursive:true});
await build({plugins:[{name:'real-plugin-build',setup(build){build.onResolve({filter:/daily-writer-plugin\/\.daily-plugin\/dist\/plugin\.js$/},()=>({path:'./writer-build/plugin.js',external:true}));}}],entryPoints:[join(repo,'scripts/fixtures/writer-task-fixture.ts')],outfile:root+'/plugin.js',bundle:true,format:'esm',platform:'browser',jsx:'automatic',nodePaths:[repo+'/node_modules'],define:{'process.env.NODE_ENV':'"production"'},alias:{'@daily/plugin-sdk':repo+'/packages/plugin-sdk/src/index.ts'},logLevel:'warning'});
await writeFile(root+'/plugin.json',JSON.stringify({identifier:'daily.writer',name:'Writer QA',entry:'plugin.js',activation:'startup',contributes:{views:[{id:'writer.workbench',title:'Writer QA',icon:'notebook-pen'}]}}));

const electron=require('electron');
const child=spawn(electron,[repo,'--remote-debugging-port='+debugPort,'--user-data-dir='+root+'/profile'],{cwd:repo,env:{...process.env,DAILY_WORKSPACE:root+'/workspace',ELECTRON_RENDERER_URL:'',ELECTRON_RUN_AS_NODE:''},stdio:['ignore','pipe','pipe']});
let log='';child.stderr.on('data',data=>log+=data);child.stdout.on('data',data=>log+=data);
const delay=ms=>new Promise(r=>setTimeout(r,ms));
async function until(fn,timeout=30000){const end=Date.now()+timeout;let error;while(Date.now()<end){try{const v=await fn();if(v)return v;}catch(e){error=e;}await delay(150);}throw error??new Error('timeout');}
class CDP {
  constructor(ws){this.ws=ws;this.id=0;this.pending=new Map();ws.addEventListener('close',()=>{for(const p of this.pending.values())p.reject(new Error('CDP closed'));this.pending.clear();});ws.addEventListener('message',e=>{const m=JSON.parse(e.data);if(m.id){const p=this.pending.get(m.id);this.pending.delete(m.id);m.error?p?.reject(new Error(JSON.stringify(m.error))):p?.resolve(m.result);}});}
  static async connect(url){const ws=new WebSocket(url);await new Promise((resolve,reject)=>{ws.addEventListener('open',resolve,{once:true});ws.addEventListener('error',reject,{once:true});});return new CDP(ws);}
  call(method,params={}){if(this.ws.readyState!==1)return Promise.reject(new Error('CDP closed'));const id=++this.id;return new Promise((resolve,reject)=>{const timer=setTimeout(()=>{this.pending.delete(id);reject(new Error('CDP timeout '+method));},20000);this.pending.set(id,{resolve:v=>{clearTimeout(timer);resolve(v);},reject:e=>{clearTimeout(timer);reject(e);}});this.ws.send(JSON.stringify({id,method,params}));});}
  async eval(expression,contextId){const r=await this.call('Runtime.evaluate',{expression,contextId,awaitPromise:true,returnByValue:true});if(r.exceptionDetails)throw new Error(JSON.stringify(r.exceptionDetails));return r.result?.value;}
}
let main;
// This creates only an isolated workspace/profile; real providers are never called.
try {
  const pages=()=>fetch(`http://127.0.0.1:${debugPort}/json/list`).then(r=>r.json());
  const page=await until(async()=>{const all=await pages(); return all.find(p=>p.type==='page'&&!p.url.includes('dailyBackground')&&p.url.includes('index.html'));});
  main=await CDP.connect(page.webSocketDebuggerUrl);
  await until(()=>main.eval('Boolean(window.dailyDesktop)'));
  async function invoke(command,input){return main.eval(`window.dailyDesktop.invoke(${JSON.stringify(command)},{input:${JSON.stringify(input??{})}}).then(r=>{if(!r.ok)throw new Error(r.error.message);return r.result;})`);}
  const config={id:'qa',name:'Local QA',providerKind:'open_ai_compatible',baseUrl:modelUrl+'/v1',models:['qa'],defaultModel:'qa',followCodex:false,apiKey:'fixture-only',apiKeyEnv:null,codexHome:null};
  await invoke('save_settings_file_content',{scope:'local',content:JSON.stringify({auth:{selectedAuthConfigId:'qa',configs:[config]},model:{model:'qa'}})});
  await invoke('install_local_dev_plugin',{manifestPath:root+'/plugin.json',devUrl:null});
  await until(()=>main.eval(`Array.from(document.querySelectorAll('button')).some(x=>x.title==='Writer QA'||x.getAttribute('aria-label')==='Writer QA')`));
  await main.eval(`Array.from(document.querySelectorAll('button')).find(x=>x.title==='Writer QA'||x.getAttribute('aria-label')==='Writer QA').click()`);
  await until(()=>main.eval('Boolean(window.__dailyTaskQA)'));
  await main.eval('window.__dailyTaskQA.seed()');
  // Reopen the view so its controller reads the seeded project.
  await main.call('Page.reload');
  await until(()=>main.eval(`Array.from(document.querySelectorAll('button')).some(x=>x.title==='Writer QA')`));
  await main.eval(`Array.from(document.querySelectorAll('button')).find(x=>x.title==='Writer QA').click()`);
  await until(()=>main.eval(`Boolean(document.querySelector('[data-plugin-view]')?.shadowRoot?.querySelector('.cm-content'))`));
  assert.equal(await main.eval(`document.querySelectorAll('iframe').length`),0);
  await main.eval(`window.__retainedView = document.querySelector('[data-plugin-view]'); window.__retainedEditor = window.__retainedView.shadowRoot.querySelector('.cm-content'); window.__retainedEditor.focus()`);
  await main.call('Input.insertText',{text:'保留的编辑状态'});
  await until(()=>main.eval(`window.__retainedEditor.innerText.includes('保留的编辑状态')`));
  const started=await main.eval('window.__dailyTaskQA.start()');
  const taskId=started.task.id;
  await until(async()=>{const s=await invoke('workspace_task',{taskId,action:'get'});if(s.status==='failed')throw new Error(s.error);return s.status==='running';});
  // Route changes hide the same view while activation keeps executing.
  await main.eval(`document.querySelector('nav button').click()`);
  assert.equal(await main.eval(`window.__retainedView.isConnected && window.__retainedView.getBoundingClientRect().width === 0`),true);
  await main.eval(`Array.from(document.querySelectorAll('button')).find(x=>x.title==='Writer QA').click()`);
  assert.equal(await main.eval(`document.querySelector('[data-plugin-view]') === window.__retainedView && window.__retainedView.shadowRoot.querySelector('.cm-content') === window.__retainedEditor && window.__retainedEditor.innerText.includes('保留的编辑状态')`),true);
  assert.equal(await main.eval(`window.__retainedView.shadowRoot.activeElement === window.__retainedEditor`),true,'switching back restores editor focus');
  await main.eval(`(document.querySelector('button[aria-label="搜索任务与命令"]') ?? Array.from(document.querySelectorAll('button')).find(x=>x.innerText.includes('搜索任务与命令'))).click()`);
  await until(()=>main.eval(`Array.from(document.querySelectorAll('[role=option]')).some(x=>/切换到(深|浅)色模式/.test(x.innerText))`));
  const theme=await main.eval(`window.__retainedView.dataset.dailyTheme`);
  await main.eval(`Array.from(document.querySelectorAll('[role=option]')).find(x=>/切换到(深|浅)色模式/.test(x.innerText)).click()`);
  await until(()=>main.eval(`window.__retainedView.dataset.dailyTheme !== ${JSON.stringify(theme)}`));
  assert.equal(await main.eval(`document.querySelector('[data-plugin-view]') === window.__retainedView && window.__retainedView.shadowRoot.querySelector('.cm-content') === window.__retainedEditor`),true,'theme changes retain the editor');
  const writerCapture=await main.call('Page.captureScreenshot');await writeFile(root+'/writer.png',Buffer.from(writerCapture.data,'base64'));
  const bounds=await main.eval(`({view:window.__retainedView.getBoundingClientRect().toJSON(), app:window.__retainedView.shadowRoot.querySelector('.w-app').getBoundingClientRect().toJSON()})`);
  assert.ok(Math.abs(bounds.app.height-bounds.view.height)<2,'view height must use its container');
  const finished=await until(async()=>{const s=await invoke('workspace_task',{taskId,action:'get'});if(['failed','interrupted'].includes(s.status))throw new Error(s.error);return s.status==='completed'&&s;},45000);
  assert.ok(finished.result.reply.length>0);
  const session=await invoke('plugin_host_rpc',{pluginId:'daily.writer',method:'agent.session.get',payload:{sessionId:started.sessionId}});
  assert.equal(session.session.entries.at(-1).content,finished.result.reply);
  // Reload the shared renderer during another operation. The sidecar journals its result.
  const beforeStats=await fetch(modelUrl+'/stats').then(r=>r.json());
  const second=await main.eval('window.__dailyTaskQA.start()');
  await until(async()=>{const stats=await fetch(modelUrl+'/stats').then(r=>r.json());return stats.requests>beforeStats.requests;});
  await main.call('Page.reload');
  await until(()=>main.eval('Boolean(window.__dailyDesktop || window.dailyDesktop)'));
  const interrupted=await until(async()=>{const task=await invoke('workspace_task',{taskId:second.task.id,action:'get'});return task.status==='interrupted'&&task;},75000);
  assert.equal(interrupted.attempt,1);
  await invoke('workspace_task',{taskId:second.task.id,action:'retry'});
  const recovered=await until(async()=>{const task=await invoke('workspace_task',{taskId:second.task.id,action:'get'});if(task.status==='failed')throw new Error(task.error);return task.status==='completed'&&task;},30000);
  assert.equal(recovered.attempt,2);
  const afterStats=await fetch(modelUrl+'/stats').then(r=>r.json());
  assert.equal(afterStats.requests,beforeStats.requests+1,'recovery must reuse the completed model operation');
  const recoveredSession=await invoke('plugin_host_rpc',{pluginId:'daily.writer',method:'agent.session.get',payload:{sessionId:second.sessionId}});
  assert.equal(recoveredSession.session.entries.length,2);
  assert.equal((await pages()).filter(p=>p.type==='page').length,1,'one renderer page per workspace');
  // Rebuild visible page once more and confirm durable snapshots and the task center.
  await main.eval(`Array.from(document.querySelectorAll('button')).find(x=>x.getAttribute('aria-label')==='任务中心'||x.getAttribute('aria-label')==='Task center').click()`);
  await until(()=>main.eval(`document.body.innerText.includes('已完成')||document.body.innerText.includes('Completed')`));
  const capture=await main.call('Page.captureScreenshot');await writeFile(root+'/task-center.png',Buffer.from(capture.data,'base64'));
  console.log(JSON.stringify({ok:true,taskId,status:finished.status,sessionEntries:session.session.entries.length,singleRenderer:true,retainedView:true,themeWithoutReload:true,focusRestored:true,realModuleChunks:true,rendererReloadRecovered:true,noDuplicateModelCalls:true,screenshot:root+'/task-center.png'}));
} catch(error){console.error(error);try{console.error("UI",await main?.eval("document.body.innerText"));console.error("TARGETS",await fetch(`http://127.0.0.1:${debugPort}/json/list`).then(r=>r.json()));}catch{}console.error(log.slice(-10000));process.exitCode=1;}
finally{main?.ws.close();child.kill('SIGKILL');await writeFile(root+'/electron.log',log);}
