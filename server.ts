import express from "express";
import path from "path";
import { createServer as createViteServer } from "vite";
import { createServer } from "http";
import { Server } from "socket.io";
import { GoogleGenAI } from "@google/genai";

// Initialize Gemini SDK
const ai = new GoogleGenAI({
  apiKey: process.env.GEMINI_API_KEY,
  httpOptions: {
    headers: {
      'User-Agent': 'aistudio-build',
    }
  }
});

async function startServer() {
  const app = express();
  const PORT = 3000;

  const httpServer = createServer(app);
  const io = new Server(httpServer, {
    cors: { origin: "*" }
  });

  const roomLanguages: Record<string, Record<string, string>> = {};

  io.on("connection", (socket) => {
    socket.on("join_room", ({ roomCode, language }) => {
      socket.join(roomCode);
      if (!roomLanguages[roomCode]) {
        roomLanguages[roomCode] = {};
      }
      roomLanguages[roomCode][socket.id] = language;
      
      socket.to(roomCode).emit("user_joined");
    });
    
    socket.on("speech_ended", async (data) => {
      const { roomCode, originalText, sourceLang, speakerId, speakerName } = data;
      
      try {
        const usersInRoom = roomLanguages[roomCode] || {};
        const otherSocketIds = Object.keys(usersInRoom).filter(id => id !== socket.id);
        
        let targetLang = sourceLang; // Default to same if alone
        if (otherSocketIds.length > 0) {
          targetLang = usersInRoom[otherSocketIds[0]];
        }
        
        const response = await ai.models.generateContent({
          model: "gemini-3.5-flash",
          contents: `Translate the following text from ${sourceLang} to ${targetLang}. Return ONLY the translated text. Do not include quotes, markdown, explanations, or any other characters. Text to translate:\n${originalText}`,
        });
        
        const translatedText = response.text?.trim() || originalText;
        
        io.to(roomCode).emit("new_message", {
          originalText,
          translatedText,
          speakerId,
          speakerName,
          sourceLang,
          targetLang
        });
      } catch (err) {
        console.error("Translation error:", err);
      }
    });

    socket.on("disconnect", () => {
      // Clean up room languages (simplified)
      for (const room in roomLanguages) {
        if (roomLanguages[room][socket.id]) {
          delete roomLanguages[room][socket.id];
        }
      }
    });
  });

  app.get("/api/health", (req, res) => {
    res.json({ status: "ok" });
  });

  if (process.env.NODE_ENV !== "production") {
    const vite = await createViteServer({
      server: { middlewareMode: true },
      appType: "spa",
    });
    app.use(vite.middlewares);
  } else {
    const distPath = path.join(process.cwd(), 'dist');
    app.use(express.static(distPath));
    app.get('*', (req, res) => {
      res.sendFile(path.join(distPath, 'index.html'));
    });
  }

  httpServer.listen(PORT, "0.0.0.0", () => {
    console.log(`Server running on http://localhost:${PORT}`);
  });
}

startServer();
