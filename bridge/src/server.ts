/**
 * WebSocket server for Python-Node.js bridge communication.
 */

import { WebSocketServer, WebSocket } from 'ws';
import { WhatsAppClient, InboundMessage } from './whatsapp.js';

interface SendCommand {
  type: 'send';
  to: string;
  text: string;
}

interface AuthCommand {
  type: 'auth';
  token: string;
}

interface BridgeMessage {
  type: 'message' | 'status' | 'qr' | 'error';
  [key: string]: unknown;
}

export class BridgeServer {
  private wss: WebSocketServer | null = null;
  private wa: WhatsAppClient | null = null;
  private clients: Set<WebSocket> = new Set();
  private authenticatedClients: WeakSet<WebSocket> = new WeakSet();
  private requireAuth: boolean;

  constructor(
    private port: number,
    private authDir: string,
    private host: string,
    private bridgeToken: string,
  ) {
    this.requireAuth = this.bridgeToken.length > 0;
  }

  async start(): Promise<void> {
    // Create WebSocket server
    this.wss = new WebSocketServer({ port: this.port, host: this.host });
    console.log(`🌉 Bridge server listening on ws://${this.host}:${this.port}`);
    if (this.requireAuth) {
      console.log('🔐 Bridge token auth is enabled');
    }

    // Initialize WhatsApp client
    this.wa = new WhatsAppClient({
      authDir: this.authDir,
      onMessage: (msg) => this.broadcast({ type: 'message', ...msg }),
      onQR: (qr) => this.broadcast({ type: 'qr', qr }),
      onStatus: (status) => this.broadcast({ type: 'status', status }),
    });

    // Handle WebSocket connections
    this.wss.on('connection', (ws) => {
      console.log('🔗 Python client connected');
      this.clients.add(ws);

      ws.on('message', async (data) => {
        try {
          const cmd = JSON.parse(data.toString()) as SendCommand | AuthCommand;
          if (cmd.type === 'auth') {
            const token = typeof cmd.token === 'string' ? cmd.token : '';
            if (!this.requireAuth || token === this.bridgeToken) {
              this.authenticatedClients.add(ws);
              ws.send(JSON.stringify({ type: 'status', status: 'authenticated' }));
            } else {
              ws.send(JSON.stringify({ type: 'error', error: 'invalid auth token' }));
              ws.close();
            }
            return;
          }

          if (this.requireAuth && !this.authenticatedClients.has(ws)) {
            ws.send(JSON.stringify({ type: 'error', error: 'authentication required' }));
            return;
          }

          await this.handleCommand(cmd);
          ws.send(JSON.stringify({ type: 'sent', to: cmd.to }));
        } catch (error) {
          console.error('Error handling command:', error);
          ws.send(JSON.stringify({ type: 'error', error: String(error) }));
        }
      });

      ws.on('close', () => {
        console.log('🔌 Python client disconnected');
        this.clients.delete(ws);
      });

      ws.on('error', (error) => {
        console.error('WebSocket error:', error);
        this.clients.delete(ws);
      });
    });

    // Connect to WhatsApp
    await this.wa.connect();
  }

  private async handleCommand(cmd: SendCommand | AuthCommand): Promise<void> {
    if (cmd.type === 'auth') {
      return;
    }
    if (cmd.type === 'send' && this.wa) {
      await this.wa.sendMessage(cmd.to, cmd.text);
    }
  }

  private broadcast(msg: BridgeMessage): void {
    const data = JSON.stringify(msg);
    for (const client of this.clients) {
      if (client.readyState === WebSocket.OPEN) {
        client.send(data);
      }
    }
  }

  async stop(): Promise<void> {
    // Close all client connections
    for (const client of this.clients) {
      client.close();
    }
    this.clients.clear();

    // Close WebSocket server
    if (this.wss) {
      this.wss.close();
      this.wss = null;
    }

    // Disconnect WhatsApp
    if (this.wa) {
      await this.wa.disconnect();
      this.wa = null;
    }
  }
}
