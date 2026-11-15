import plugin from "../../../daily-writer-plugin/.daily-plugin/dist/plugin.js";
import { setDailyHost } from "../../packages/plugin-sdk/src/index.ts";
import { saveLastProjectId } from "../../../daily-writer-plugin/src/storage/prefs.ts";
export default {
  views: plugin.views,
  activate(host) { setDailyHost(host); return plugin.activate(host); },
};
import { createProject } from "../../../daily-writer-plugin/src/storage/projectStore.ts";
import { dailyHost } from "../../packages/plugin-sdk/src/index.ts";
import { submitWriterTask } from "../../../daily-writer-plugin/src/worker/taskClient.ts";
window.__dailyTaskQA = {
  async seed() {
    const project = await createProject({ title: "插件切换验收", outline: "主角从北境出发寻找朋友。" });
    await saveLastProjectId(project.id);
    return project.id;
  },
  async start() {
    const project=await createProject({title:"后台任务验收",outline:"主角从北境出发寻找失踪的朋友。"});
    const session=await dailyHost().agent.session.create({scope:project.id});
    const task=await submitWriterTask("writer.assistant",project.id,{project,texts:{},sessionId:session.id,mode:"agent",instruction:"请概括目前的故事",activeDoc:{kind:"outline"},selection:null},`assistant:${session.id}`);
    return {projectId:project.id,sessionId:session.id,task};
  },
  get:(id)=>dailyHost().tasks.get(id),
  session:(id)=>dailyHost().agent.session.get(id),
};
