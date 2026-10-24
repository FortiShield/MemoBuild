# Base builder Dockerfile for MemoBuild
# Used for development and CI builds
FROM node:18-alpine AS builder

WORKDIR /app

# Install dependencies
COPY package.json .
RUN npm install

# Copy source and build
COPY src ./src
RUN npm run build

# Production-ready configuration
ENV NODE_ENV=production
EXPOSE 3000
CMD ["npm", "start"]
